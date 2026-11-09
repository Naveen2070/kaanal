//! The live generator (step 8L, RULES 105): Poisson arrivals at the offered rate, sampled into
//! plans from the current generation and chaos and submitted to the shared engine actor. A change
//! of rate, a pause and a stop take effect at the next loop turn; an edit of the system is seen at
//! the next arrival because each arrival reads the generation and the chaos afresh.
//!
//! Pacing is by catch-up: the loop keeps the instant of the next arrival and submits every arrival
//! due at each turn, so a coarse timer tick (about 15 ms on Windows) does not cap the rate. A turn
//! submits at most [`BURST`] arrivals; a generator that falls further behind drops the backlog
//! rather than flood the engine. Two caps protect the server: a rate above [`MAX_RATE`] is refused,
//! and with [`MAX_IN_FLIGHT`] requests unfinished no more are generated until the engine catches
//! up (an unbounded queue under overload would otherwise grow without limit).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use kaanal_core::replay::Finished;
use kaanal_core::{ChainRequest, sample_plan};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use crate::state::AppState;

/// Highest offered rate a live run accepts, requests a second.
pub const MAX_RATE: f64 = 20_000.0;
/// Most unfinished generated requests before the generator stops adding more.
pub const MAX_IN_FLIGHT: u64 = 200_000;
/// Most arrivals one loop turn submits.
const BURST: u32 = 20_000;
/// The longest the loop sleeps between turns.
const TURN: Duration = Duration::from_millis(10);

/// The caps; tests lower them.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_rate: f64,
    pub max_in_flight: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_rate: MAX_RATE,
            max_in_flight: MAX_IN_FLIGHT,
        }
    }
}

/// One route of the offered mix: its share of the rate and its body sizes.
#[derive(Debug, Clone)]
pub struct StreamShare {
    pub endpoint_key: String,
    /// Relative weight; the rate is split in proportion.
    pub share: f64,
    pub req_bytes: u64,
    pub resp_bytes: u64,
}

/// A rate the generator refuses, with the reason for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateRefused(pub String);

/// What the generator has done so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenStats {
    pub submitted: u64,
    pub completed: u64,
    /// Submitted and not yet finished.
    pub in_flight: u64,
    /// The in-flight cap was hit and arrivals are being skipped.
    pub overloaded: bool,
    pub rate: f64,
    pub paused: bool,
}

/// State shared between the handle, the arrival loop and the drain task.
struct Ctl {
    rate_bits: AtomicU64,
    paused: AtomicBool,
    stopped: AtomicBool,
    overloaded: AtomicBool,
    submitted: AtomicU64,
    completed: AtomicU64,
    limits: Limits,
}

impl Ctl {
    fn in_flight(&self) -> u64 {
        self.submitted
            .load(Ordering::Relaxed)
            .saturating_sub(self.completed.load(Ordering::Relaxed))
    }
}

/// A running generator; clones share it.
#[derive(Clone)]
pub struct GeneratorHandle {
    ctl: Arc<Ctl>,
}

impl GeneratorHandle {
    /// Sets the offered rate (requests a second, all routes together).
    ///
    /// # Errors
    /// [`RateRefused`] for a rate that is negative, not a number, or above the cap; the rate in
    /// force is left as it was.
    pub fn set_rate(&self, rps: f64) -> Result<(), RateRefused> {
        if !rps.is_finite() || rps < 0.0 {
            return Err(RateRefused(
                "the rate must be a number of at least 0".into(),
            ));
        }
        if rps > self.ctl.limits.max_rate {
            return Err(RateRefused(format!(
                "a live run takes at most {} requests a second",
                self.ctl.limits.max_rate
            )));
        }
        self.ctl.rate_bits.store(rps.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    /// Stops generating (the engine keeps serving and draining what is in it).
    pub fn pause(&self) {
        self.ctl.paused.store(true, Ordering::Relaxed);
    }

    /// Starts generating again, from now (a pause leaves no backlog to catch up).
    pub fn resume(&self) {
        self.ctl.paused.store(false, Ordering::Relaxed);
    }

    /// Ends the generator; its loop and drain task finish.
    pub fn stop(&self) {
        self.ctl.stopped.store(true, Ordering::Relaxed);
    }

    /// Whether [`GeneratorHandle::stop`] was called.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.ctl.stopped.load(Ordering::Relaxed)
    }

    /// Counts so far.
    #[must_use]
    pub fn stats(&self) -> GenStats {
        GenStats {
            submitted: self.ctl.submitted.load(Ordering::Relaxed),
            completed: self.ctl.completed.load(Ordering::Relaxed),
            in_flight: self.ctl.in_flight(),
            overloaded: self.ctl.overloaded.load(Ordering::Relaxed),
            rate: f64::from_bits(self.ctl.rate_bits.load(Ordering::Relaxed)),
            paused: self.ctl.paused.load(Ordering::Relaxed),
        }
    }
}

/// Starts a generator over `mix` at `rps` on the engine of `st`'s current generation. Every
/// finished generated request is counted and forwarded to `forward` when given.
#[must_use]
pub fn start(
    st: Arc<AppState>,
    mix: Vec<StreamShare>,
    rps: f64,
    seed: u64,
    limits: Limits,
    forward: Option<mpsc::UnboundedSender<Finished>>,
) -> GeneratorHandle {
    let ctl = Arc::new(Ctl {
        rate_bits: AtomicU64::new(rps.clamp(0.0, limits.max_rate).to_bits()),
        paused: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        overloaded: AtomicBool::new(false),
        submitted: AtomicU64::new(0),
        completed: AtomicU64::new(0),
        limits,
    });
    let (tap, mut rx) = mpsc::unbounded_channel::<Finished>();
    if let Some(engine) = st.generation().and_then(|g| g.engine.clone()) {
        engine.tap(Some(tap.clone()));
    }
    let drain = Arc::clone(&ctl);
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            // a real mock request rides the same tap (step 8A) but was never submitted here
            if crate::actor::is_generated(&f) {
                drain.completed.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(to) = &forward {
                let _ = to.send(f);
            }
            if drain.stopped.load(Ordering::Relaxed) && drain.in_flight() == 0 {
                return;
            }
        }
    });
    tokio::spawn(arrivals(st, mix, seed, Arc::clone(&ctl)));
    GeneratorHandle { ctl }
}

/// **Private.** The arrival loop.
async fn arrivals(st: Arc<AppState>, mix: Vec<StreamShare>, seed: u64, ctl: Arc<Ctl>) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let total: f64 = mix.iter().map(|m| m.share).sum();
    let mut next = t0;
    let mut n = 0u64;
    loop {
        if ctl.stopped.load(Ordering::Relaxed) {
            return;
        }
        let rps = f64::from_bits(ctl.rate_bits.load(Ordering::Relaxed));
        let now = Instant::now();
        if ctl.paused.load(Ordering::Relaxed) || rps <= 0.0 || mix.is_empty() || total <= 0.0 {
            // no backlog builds while idle: the next arrival is drawn from the moment of resuming
            next = now + Duration::from_millis(50);
            sleep_until(next).await;
            continue;
        }
        let mut burst = 0;
        while next <= now && burst < BURST {
            if ctl.in_flight() >= ctl.limits.max_in_flight {
                ctl.overloaded.store(true, Ordering::Relaxed);
            } else {
                if ctl.in_flight() < ctl.limits.max_in_flight / 10 * 9 {
                    ctl.overloaded.store(false, Ordering::Relaxed);
                }
                arrive(&st, &mix, total, &mut rng, n, &ctl);
                n += 1;
            }
            next += gap(rng.random::<f64>(), rps);
            burst += 1;
        }
        if burst >= BURST {
            next = now; // far behind: drop the backlog
        }
        sleep_until(next.min(now + TURN)).await;
    }
}

/// The longest wait between two arrivals: a tiny rate would otherwise overflow the instant.
const MAX_GAP: Duration = Duration::from_secs(3600);

/// **Private.** The exponential gap to the next arrival for a uniform draw `u` in `[0, 1)` at
/// `rps` a second; `1 - u` keeps the logarithm finite, and it never exceeds [`MAX_GAP`].
fn gap(u: f64, rps: f64) -> Duration {
    let secs = -(1.0 - u).ln() / rps;
    Duration::try_from_secs_f64(secs).map_or(MAX_GAP, |d| d.min(MAX_GAP))
}

/// **Private.** One arrival: a route drawn by share, a plan sampled from the current generation
/// and chaos, submitted to its engine.
fn arrive(st: &AppState, mix: &[StreamShare], total: f64, rng: &mut StdRng, n: u64, ctl: &Ctl) {
    let Some(g) = st.generation() else { return };
    let Some(engine) = g.engine.as_ref() else {
        return;
    };
    let mut x = rng.random::<f64>() * total;
    let m = mix
        .iter()
        .find(|m| {
            x -= m.share;
            x < 0.0
        })
        .unwrap_or(&mix[mix.len() - 1]);
    let Some(ep) = g.endpoints.get(&m.endpoint_key) else {
        return;
    };
    let chaos = st.snapshot();
    let (plan, _) = sample_plan(
        &g.topo,
        &chaos.chaos,
        &ChainRequest {
            endpoint: ep,
            // the same clock as the mock: the time since the system started
            at: g.started.elapsed(),
            n,
            seed: g.chain_seed,
            pin_delay: Duration::ZERO,
            req_bytes: m.req_bytes,
            resp_bytes: m.resp_bytes,
        },
    );
    engine.submit(Arc::new(plan));
    ctl.submitted.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::Config;
    use kaanal_core::replay::Engine;

    const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
";

    /// Review finding 5: a tiny positive rate must not overflow the next-arrival instant.
    #[test]
    fn the_gap_between_arrivals_is_capped_so_a_tiny_rate_cannot_overflow_an_instant() {
        let g = gap(0.5, 1e-12);
        assert!(g <= Duration::from_secs(3600), "{g:?}");
        let mut at = Instant::now();
        at += g;
        assert_eq!(gap(0.5, 100.0).as_secs_f64().round(), 0.0);
    }

    fn state(yaml: &str) -> Arc<AppState> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let cfg = Config::from_yaml(yaml).unwrap();
        let mut st = AppState::build(&cfg, &dir, "t".into(), 0).unwrap();
        let g = st.gen_mut();
        g.engine = Some(crate::actor::spawn(Engine::live(&g.topo)));
        Arc::new(st)
    }

    fn mix() -> Vec<StreamShare> {
        vec![StreamShare {
            endpoint_key: "GET /users/{id}".into(),
            share: 1.0,
            req_bytes: 0,
            resp_bytes: 0,
        }]
    }

    #[tokio::test(start_paused = true)]
    async fn arrivals_follow_the_rate() {
        let g = start(state(YAML), mix(), 1000.0, 7, Limits::default(), None);
        tokio::time::sleep(Duration::from_secs(10)).await;
        let s = f64::from(u32::try_from(g.stats().submitted).unwrap());
        assert!(
            (s - 10_000.0).abs() / 10_000.0 < 0.05,
            "{s} arrivals in 10 s at 1000/s"
        );
        g.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn pause_stops_new_work_and_resume_restarts_it() {
        let g = start(state(YAML), mix(), 500.0, 7, Limits::default(), None);
        tokio::time::sleep(Duration::from_secs(2)).await;
        g.pause();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let at_pause = g.stats().submitted;
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(
            g.stats().submitted,
            at_pause,
            "nothing is generated while paused"
        );
        assert!(g.stats().paused);
        g.resume();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let after = g.stats().submitted - at_pause;
        // no catch-up for the paused 5 s: about 2 s of arrivals at 500/s
        assert!((800..1200).contains(&after), "{after}");
        g.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_above_the_cap_is_refused_and_the_run_unchanged() {
        let g = start(state(YAML), mix(), 100.0, 7, Limits::default(), None);
        assert!(g.set_rate(20_001.0).is_err());
        assert!(g.set_rate(f64::NAN).is_err());
        assert!(g.set_rate(-1.0).is_err());
        assert_eq!(g.stats().rate, 100.0);
        assert_eq!(g.set_rate(20_000.0), Ok(()));
        assert_eq!(g.stats().rate, 20_000.0);
        g.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn overload_stops_the_generator_at_the_in_flight_cap_and_flags_it() {
        // a database that finishes one call a second under 100 a second: nothing completes in time
        let slow = YAML.replace(
            "db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }",
            "db: { type: database, concurrency: 1, ops: { read: 1s, write: 1s, scan: 1s } }",
        );
        let limits = Limits {
            max_rate: MAX_RATE,
            max_in_flight: 50,
        };
        let g = start(state(&slow), mix(), 100.0, 7, limits, None);
        tokio::time::sleep(Duration::from_secs(3)).await;
        let s = g.stats();
        assert!(s.overloaded, "{s:?}");
        assert!(s.in_flight <= 50 + 3, "memory stays bounded: {s:?}");
        g.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn rate_zero_idles() {
        let g = start(state(YAML), mix(), 0.0, 7, Limits::default(), None);
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(g.stats().submitted, 0);
        g.set_rate(200.0).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(g.stats().submitted > 300);
        g.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn finished_requests_are_counted_and_forwarded() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let g = start(state(YAML), mix(), 200.0, 7, Limits::default(), Some(tx));
        tokio::time::sleep(Duration::from_secs(2)).await;
        g.pause();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let s = g.stats();
        assert_eq!(
            s.completed, s.submitted,
            "everything generated has finished"
        );
        let mut got = 0u64;
        while rx.try_recv().is_ok() {
            got += 1;
        }
        assert_eq!(got, s.completed);
        g.stop();
    }

    /// Step 8A: real mock requests are forwarded to the session but never counted as generated,
    /// so the in-flight cap and the overload flag are about generated load only.
    #[tokio::test(start_paused = true)]
    async fn real_requests_are_forwarded_but_not_counted_as_generated() {
        let st = state(YAML);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let g = start(Arc::clone(&st), mix(), 0.0, 7, Limits::default(), Some(tx));
        for _ in 0..3 {
            let req = axum::http::Request::builder()
                .method("GET")
                .uri("/users/1")
                .body(axum::body::Body::empty())
                .unwrap();
            let r = crate::mock::handle(axum::extract::State(Arc::clone(&st)), req).await;
            assert_eq!(r.status(), 200);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = g.stats();
        assert_eq!((s.submitted, s.completed, s.in_flight), (0, 0, 0));
        let mut got = 0;
        while rx.try_recv().is_ok() {
            got += 1;
        }
        assert_eq!(got, 3, "the session still sees the real requests");
        g.stop();
    }
}
