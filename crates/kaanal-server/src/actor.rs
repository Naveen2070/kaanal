//! The live driver of the replay engine: one task owns the [`Engine`] and a catch-up clock.
//!
//! Handlers send a sampled plan and await its [`Finished`]. The task sleeps until the engine's
//! next event, then runs every event that is due **at its scheduled time** (`advance_to`), so a
//! late wake-up (the Windows timer ticks about every 15 ms) delays only the answer, never the
//! hop starts of the requests behind it. An event already due when the loop comes round (a
//! zero-length hop) runs without touching the timer, so a lone request has no timer floor; a
//! queued arrival still goes before it (ties favour the arrival). A request whose client went
//! away stays in the engine until it ends, like a call still running on a real server; its
//! answer is dropped.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::replay::{Engine, Finished};
use kaanal_core::{Plan, Topology};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};

/// **Private.** One message for the engine task.
enum Cmd {
    /// Run a plan.
    Run {
        plan: Arc<Plan>,
        reply: oneshot::Sender<Finished>,
    },
    /// Refill the rate-limit buckets (`POST /reset`).
    Refill,
    /// Change the running engine's parameters in place (step 8L); the reply is the engine's answer.
    Reconfigure {
        topo: Arc<Topology>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Run a generated plan: no reply, the finished request goes to the tap.
    Submit { plan: Arc<Plan> },
    /// Where the finished requests of generated plans go (`None` drops them).
    Tap(Option<mpsc::UnboundedSender<Finished>>),
    /// `(running, waiting)` of every node now.
    Loads {
        reply: oneshot::Sender<Vec<(u32, usize)>>,
    },
}

/// **Private.** Marks the engine tag of a generated plan, so its answer goes to the tap and not to
/// a waiting handler.
const GENERATED: u64 = 1 << 63;

/// Whether `f` is a generated request (the live generator's) and not a real mock request.
#[must_use]
pub fn is_generated(f: &Finished) -> bool {
    f.tag & GENERATED != 0
}

/// A handle to the engine task; clones share it.
#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl EngineHandle {
    /// Runs `plan` on the engine clock and returns what happened. `None` when the engine task
    /// has stopped (a clock overflow, or the runtime is shutting down).
    pub async fn run(&self, plan: Arc<Plan>) -> Option<Finished> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Cmd::Run { plan, reply }).ok()?;
        rx.await.ok()
    }
}

impl EngineHandle {
    /// Changes the running engine's parameters in place (step 8L, `Engine::reconfigure`): queues,
    /// running work and breaker state carry over. A change of structure is refused with the
    /// engine's reason, and so is a stopped engine.
    ///
    /// # Errors
    /// The reason the engine refused the change, or that it has stopped.
    pub async fn reconfigure(&self, topo: Arc<Topology>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Reconfigure { topo, reply })
            .map_err(|_| "the engine has stopped".to_string())?;
        rx.await.map_err(|_| "the engine has stopped".to_string())?
    }

    /// Runs a generated plan without waiting for it (live traffic, step 8L): its finished request
    /// goes to the tap set with [`EngineHandle::tap`].
    pub fn submit(&self, plan: Arc<Plan>) {
        let _ = self.tx.send(Cmd::Submit { plan });
    }

    /// `(running, waiting)` of every node at this instant (`None` when the engine has stopped).
    pub async fn loads(&self) -> Option<Vec<(u32, usize)>> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Cmd::Loads { reply }).ok()?;
        rx.await.ok()
    }

    /// Sets where generated plans' finished requests go; `None` drops them.
    pub fn tap(&self, to: Option<mpsc::UnboundedSender<Finished>>) {
        let _ = self.tx.send(Cmd::Tap(to));
    }

    /// Refills the engine's rate-limit buckets so a seeded run can repeat with limits. Fire and
    /// forget, so it can be called under the control lock; the channel is FIFO, so every request
    /// *started* after this returns sees full buckets (events already due are run first, so they
    /// do not take post-refill tokens). A request already in flight keeps its slots and queue
    /// places (the gates are not reset: that would leak or double-free a slot) and finishes
    /// normally; but its *later* hops at a rate-limited node take tokens from the refilled
    /// bucket. The bucket is time-based (GCRA), so a repeat of a run needs the same wall-clock
    /// pacing after the reset, not just the same request order.
    pub fn refill_buckets(&self) {
        let _ = self.tx.send(Cmd::Refill);
    }
}

/// Spawns the engine task on the current runtime.
pub fn spawn(engine: Engine) -> EngineHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(drive(engine, rx));
    EngineHandle { tx }
}

/// **Private.** The task: wait for a command or the next due event, advance, answer.
async fn drive(mut engine: Engine, mut rx: mpsc::UnboundedReceiver<Cmd>) {
    let t0 = Instant::now();
    let now = || u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let mut waiting: HashMap<u64, oneshot::Sender<Finished>> = HashMap::new();
    let mut next_tag = 0u64;
    let mut tap: Option<mpsc::UnboundedSender<Finished>> = None;
    loop {
        let due = engine.next_due();
        // An event already due (a zero-length hop) wakes at once: `sleep_until` would round it up
        // to the next timer tick (about 15 ms on Windows), a floor on every lone request. It stays
        // the second select branch, so a queued arrival still goes first. No spin: `advance_to(now)`
        // runs every event due at or before `now`, so the next `due` is later or the engine failed.
        let already_due = due.is_some_and(|d| d <= now());
        let wake = async move {
            match due {
                _ if already_due => {}
                Some(d) => sleep_until(t0 + Duration::from_nanos(d)).await,
                None => std::future::pending().await,
            }
        };
        // Biased: an arrival already queued at the instant an event is due goes first (ties favour the arrival, as in the simulator).
        let ok = tokio::select! {
            biased;
            cmd = rx.recv() => match cmd {
                None => return,
                Some(Cmd::Refill) => {
                    // Run what is already due first, so past-due hops take their tokens
                    // before the refill and not from the fresh bucket.
                    let ok = engine.advance_to(now()).is_ok();
                    engine.refill_buckets();
                    ok
                }
                Some(Cmd::Reconfigure { topo, reply }) => {
                    let result = engine.reconfigure(now(), &topo).map_err(|e| e.to_string());
                    let _ = reply.send(result);
                    true
                }
                Some(Cmd::Submit { plan }) => {
                    next_tag += 1;
                    engine.submit(now(), plan, GENERATED | next_tag).is_ok()
                }
                Some(Cmd::Loads { reply }) => {
                    let _ = reply.send(engine.node_loads());
                    true
                }
                Some(Cmd::Tap(to)) => {
                    tap = to;
                    true
                }
                Some(Cmd::Run { plan, reply }) => {
                    let t = now();
                    next_tag += 1;
                    waiting.insert(next_tag, reply);
                    // `submit` runs every event due strictly before `t` itself; an event exactly at `t` stays queued, so an arrival wins a tie as in the simulator.
                    engine.submit(t, plan, next_tag).is_ok()
                }
            },
            () = wake => engine.advance_to(now()).is_ok(),
        };
        for f in engine.drain_finished_iter() {
            if let Some(reply) = waiting.remove(&f.tag) {
                // a real request: the live session reads a copy (step 8A), while one holds the tap
                if let Some(to) = &tap
                    && to.send(f.clone()).is_err()
                {
                    tap = None; // the session ended
                }
                let _ = reply.send(f); // the client may be gone
            } else if is_generated(&f)
                && let Some(to) = &tap
                && to.send(f).is_err()
            {
                tap = None; // the session ended
            }
        }
        if !ok {
            return; // dropping `waiting` makes every pending handler see `None`
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::chain::{EndpointsInput, RouteSpec};
    use kaanal_core::{ChainRequest, Config, EffectiveChaos, Topology, compile, sample_plan};

    const YAML: &str = "version: 1\nnodes:\n  gw: { type: gateway, proc: 2ms, concurrency: 1 }\n  app: { type: service, proc: 3ms }\nedges:\n  - { from: gw, to: app, net: 1ms }\n";

    fn plans(n: u64) -> (Topology, Vec<Arc<Plan>>) {
        plans_for(YAML, n)
    }

    fn plans_for(yaml: &str, n: u64) -> (Topology, Vec<Arc<Plan>>) {
        let cfg = Config::from_yaml(yaml).unwrap();
        let t = Topology::build(&cfg).unwrap();
        let eps = compile(
            &t,
            &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
            EndpointsInput::default(),
        )
        .unwrap();
        let ep = eps.get("GET /x/{id}").unwrap();
        let plans = (0..n)
            .map(|i| {
                Arc::new(
                    sample_plan(
                        &t,
                        &EffectiveChaos::default(),
                        &ChainRequest {
                            at: std::time::Duration::ZERO,
                            endpoint: ep,
                            n: i,
                            seed: 1,
                            pin_delay: std::time::Duration::ZERO,
                            req_bytes: 0,
                            resp_bytes: 0,
                        },
                    )
                    .0,
                )
            })
            .collect();
        (t, plans)
    }

    /// Scripted arrivals every 1 ms through the actor (paused clock) and through the engine
    /// directly give the same answers: the actor adds no ordering, clock or reply error.
    #[tokio::test(start_paused = true)]
    async fn the_actor_gives_the_same_records_as_the_engine_driven_directly() {
        let (t, plans) = plans(20);
        let mut direct = Engine::new(&t);
        let mut want = Vec::new();
        for (i, p) in plans.iter().enumerate() {
            let at = i as u64 * 1_000_000;
            direct.submit(at, Arc::clone(p), i as u64).unwrap();
            want.extend(direct.drain_finished());
        }
        direct.advance_to(u64::MAX / 2).unwrap();
        want.extend(direct.drain_finished());
        want.sort_by_key(|f| f.order);

        let h = spawn(Engine::new(&t));
        let mut tasks = Vec::new();
        for p in &plans {
            let (h, p) = (h.clone(), Arc::clone(p));
            tasks.push(tokio::spawn(async move { h.run(p).await.unwrap() }));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let mut got = Vec::new();
        for t in tasks {
            got.push(t.await.unwrap());
        }
        assert_eq!(got.len(), want.len());
        // Tags differ (the actor numbers its own); everything else must match.
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(
                (g.arrival, g.done, &g.result, &g.hops),
                (w.arrival, w.done, &w.result, &w.hops)
            );
        }
    }

    /// An arrival at the exact instant a slot frees is served after the arrival, as in the
    /// engine and the simulator: with no queue it overflows (the release has not run yet).
    #[tokio::test(start_paused = true)]
    async fn an_arrival_at_the_release_instant_overflows_like_the_simulator() {
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 2ms, concurrency: { limit: 1, queue: 0 } }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n";
        let (t, plans) = plans_for(yaml, 2);
        let h = spawn(Engine::new(&t));
        let first = tokio::spawn({
            let (h, p) = (h.clone(), Arc::clone(&plans[0]));
            async move { h.run(p).await.unwrap() }
        });
        tokio::time::sleep(Duration::from_millis(2)).await; // exactly when the first hop ends
        let second = h.run(Arc::clone(&plans[1])).await.unwrap();
        assert_eq!(first.await.unwrap().result, kaanal_core::ChainResult::Ok);
        assert!(
            matches!(
                second.result,
                kaanal_core::ChainResult::Err { status: 503, .. }
            ),
            "{:?}",
            second.result
        );
    }
    #[tokio::test(start_paused = true)]
    async fn a_dropped_receiver_does_not_stop_the_engine() {
        let (t, plans) = plans(2);
        let h = spawn(Engine::new(&t));
        let (h2, p) = (h.clone(), Arc::clone(&plans[0]));
        let gone = tokio::spawn(async move { h2.run(p).await });
        tokio::time::sleep(Duration::from_millis(1)).await;
        gone.abort();
        let f = h
            .run(Arc::clone(&plans[1]))
            .await
            .expect("the engine still runs");
        assert!(f.done > f.arrival);
    }

    /// A hop already due when the refill arrives takes its token from the old bucket, not the
    /// fresh one: the actor runs due events before refilling. (`advance` moves the paused clock
    /// without letting the actor run, so the refill is queued while A's hop is due but unrun.)
    #[tokio::test(start_paused = true)]
    async fn the_refill_runs_already_due_events_before_it_refills() {
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 10ms }\n  app: { type: service, proc: 1ms, rate_limit: { rps: 1, burst: 1 } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n";
        let (t, plans) = plans_for(yaml, 2);
        let h = spawn(Engine::new(&t));
        let a = tokio::spawn({
            let (h, p) = (h.clone(), Arc::clone(&plans[0]));
            async move { h.run(p).await.unwrap() }
        });
        tokio::time::sleep(Duration::from_millis(1)).await; // A is in; its app hop is due at ~11 ms
        tokio::time::advance(Duration::from_millis(19)).await;
        h.refill_buckets();
        let b = h.run(Arc::clone(&plans[1])).await.unwrap();
        assert_eq!(a.await.unwrap().result, kaanal_core::ChainResult::Ok);
        // A spent the old bucket's token before the refill, so B finds a full bucket.
        assert_eq!(b.result, kaanal_core::ChainResult::Ok, "{:?}", b.result);
    }
    /// Zero-duration hops, so the benchmark measures the actor and not the sleeps.
    const YAML0: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n";

    /// Three arrivals at one instant on a zero-duration `{ limit: 1, queue: 1 }` gateway give
    /// the engine's answers (the first runs, the second queues, the third overflows): an event
    /// already due never jumps a queued arrival. `join!` sends all three before the actor runs.
    #[tokio::test(start_paused = true)]
    async fn simultaneous_arrivals_beat_an_already_due_release_like_the_engine() {
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway, concurrency: { limit: 1, queue: 1 } }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n";
        let (t, plans) = plans_for(yaml, 3);
        let mut direct = Engine::new(&t);
        for (i, p) in plans.iter().enumerate() {
            direct.submit(0, Arc::clone(p), i as u64).unwrap();
        }
        direct.advance_to(u64::MAX / 2).unwrap();
        let mut want = direct.drain_finished();
        want.sort_by_key(|f| f.order);

        let h = spawn(Engine::new(&t));
        let (first, second, third) = tokio::join!(
            h.run(Arc::clone(&plans[0])),
            h.run(Arc::clone(&plans[1])),
            h.run(Arc::clone(&plans[2]))
        );
        let got = [first.unwrap(), second.unwrap(), third.unwrap()];
        assert!(
            matches!(
                want[2].result,
                kaanal_core::ChainResult::Err { status: 503, .. }
            ),
            "{:?}",
            want[2].result
        );
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(
                (g.done - g.arrival, &g.result, &g.hops),
                (w.done - w.arrival, &w.result, &w.hops)
            );
        }
    }

    /// A lone request through a zero-duration topology does not wait for a timer tick (about
    /// 15 ms on Windows, 1 ms elsewhere). Real clock; the median of 50 sequential requests keeps
    /// a loaded runner's outliers out, and 10 ms is far above the actor's microseconds.
    #[tokio::test]
    async fn a_lone_zero_duration_request_has_no_timer_floor() {
        let (t, plans) = plans_for(YAML0, 1);
        let h = spawn(Engine::new(&t));
        let mut took: Vec<Duration> = Vec::new();
        for _ in 0..50 {
            let start = std::time::Instant::now();
            h.run(Arc::clone(&plans[0])).await.unwrap();
            took.push(start.elapsed());
        }
        took.sort();
        assert!(
            took[25] < Duration::from_millis(10),
            "median {:?}",
            took[25]
        );
    }

    /// `cargo test -p kaanal-server --release --lib -- --ignored --nocapture actor_throughput`
    #[tokio::test]
    #[ignore = "benchmark; run with --release"]
    async fn actor_throughput() {
        let (t, plans) = plans_for(YAML0, 1);
        let h = spawn(Engine::new(&t));
        let n = 200_000u32;
        let start = std::time::Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (h, p) = (h.clone(), Arc::clone(&plans[0]));
            tasks.push(tokio::spawn(async move {
                for _ in 0..n / 8 {
                    h.run(Arc::clone(&p)).await.unwrap();
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        eprintln!(
            "{:.0} requests/s through the actor (limit-free plan, zero-duration hops)",
            f64::from(n) / start.elapsed().as_secs_f64()
        );
    }

    /// Step 8L: a parameter change reaches the running engine and a structural one is refused.
    #[tokio::test(start_paused = true)]
    async fn a_reconfigure_reaches_the_engine_and_a_structural_one_is_refused() {
        let slow = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: 50ms }\nedges:\n  - { from: gw, to: app }\n";
        let (t, plans) = plans_for(slow, 3);
        let h = spawn(Engine::live(&t));
        let mut runs = Vec::new();
        for p in &plans {
            let (h, p) = (h.clone(), Arc::clone(p));
            runs.push(tokio::spawn(async move { h.run(p).await.unwrap() }));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        // three slots at 10 ms: the two that waited start at once and end at 60 ms, not 100 and 150
        let (t3, _) = plans_for(&slow.replace("concurrency: 1", "concurrency: 3"), 1);
        assert_eq!(h.reconfigure(Arc::new(t3)).await, Ok(()));
        let mut done = Vec::new();
        for r in runs {
            done.push(r.await.unwrap().done);
        }
        done.sort_unstable();
        assert_eq!(done, vec![50_000_000, 60_000_000, 60_000_000]);
        let more = slow.replace(
            "edges:",
            "  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:",
        );
        let (t_more, _) = plans_for(&more, 1);
        let refused = h.reconfigure(Arc::new(t_more)).await;
        assert!(refused.is_err(), "{refused:?}");
    }

    /// Generated traffic is submitted without a reply and its finished requests go to the tap;
    /// a mock request still gets its own answer (and, since step 8A, a copy goes to the tap too).
    #[tokio::test(start_paused = true)]
    async fn submit_delivers_finished_to_the_tap_and_not_to_a_waiter() {
        let (t, plans) = plans(3);
        let h = spawn(Engine::live(&t));
        let (tx, mut rx) = mpsc::unbounded_channel();
        h.tap(Some(tx));
        h.submit(Arc::clone(&plans[0]));
        h.submit(Arc::clone(&plans[1]));
        let mock = h.run(Arc::clone(&plans[2])).await.unwrap();
        assert!(mock.done > mock.arrival);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut got = Vec::new();
        while let Ok(f) = rx.try_recv() {
            got.push(f);
        }
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|f| f.done > f.arrival));
        assert_eq!(got.iter().filter(|f| is_generated(f)).count(), 2);
    }

    /// Step 8A: a real request answers its handler and also reaches the tap, as a copy a live
    /// session reads; a generated plan reaches only the tap.
    #[tokio::test(start_paused = true)]
    async fn a_real_request_reaches_the_tap_and_still_answers_its_handler() {
        let (t, plans) = plans(2);
        let h = spawn(Engine::new(&t));
        let (tx, mut rx) = mpsc::unbounded_channel();
        h.tap(Some(tx));
        h.submit(Arc::clone(&plans[0]));
        let real = h.run(Arc::clone(&plans[1])).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut seen = Vec::new();
        while let Ok(f) = rx.try_recv() {
            seen.push(f);
        }
        assert_eq!(seen.len(), 2);
        assert_eq!(seen.iter().filter(|f| is_generated(f)).count(), 1);
        let copy = seen.iter().find(|f| !is_generated(f)).unwrap();
        assert_eq!((copy.arrival, copy.done), (real.arrival, real.done));
        assert_eq!(copy.hops, real.hops);
    }

    #[tokio::test(start_paused = true)]
    async fn a_real_request_after_the_tap_closed_still_answers() {
        let (t, plans) = plans(2);
        let h = spawn(Engine::new(&t));
        let (tx, rx) = mpsc::unbounded_channel();
        h.tap(Some(tx));
        drop(rx);
        assert!(h.run(Arc::clone(&plans[0])).await.is_some());
        assert!(
            h.run(Arc::clone(&plans[1])).await.is_some(),
            "the tap went away, the mock did not"
        );
    }
}
