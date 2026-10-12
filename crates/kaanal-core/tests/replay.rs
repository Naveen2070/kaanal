//! The replay engine and its parts: `Bucket` and `Gate` (Task 2), `Engine` (Tasks 3 and 4).

use kaanal_core::replay::{Admit, Bucket, Engine, EngineError, Finished, Gate};
use kaanal_core::topology::{Admission, RateLimit};
use kaanal_core::{ChainResult, Config, Hop, HopKind, NodeId, Plan, PlanItem, Topology};
use std::sync::Arc;
use std::time::Duration;

const MS: u64 = 1_000_000;

fn bucket(interval_ms: u64, burst: u32) -> Bucket {
    Bucket::new(&RateLimit {
        interval_ns: interval_ms * MS,
        burst,
    })
}

fn gate(slots: u32, queue: Option<u32>) -> Gate {
    Gate::new(&Admission {
        slots,
        queue,
        overflow_status: 503,
    })
}

#[test]
fn bucket_with_burst_one_allows_one_then_asks_for_a_full_interval() {
    let mut b = bucket(10, 1);
    assert_eq!(b.take(0), Ok(()));
    assert_eq!(b.take(0), Err(10 * MS));
    assert_eq!(b.take(4 * MS), Err(6 * MS));
    assert_eq!(b.take(10 * MS), Ok(()));
}

#[test]
fn bucket_starts_full_and_refills_one_token_per_interval() {
    let mut b = bucket(10, 3);
    for _ in 0..3 {
        assert_eq!(b.take(0), Ok(()));
    }
    assert_eq!(b.take(0), Err(10 * MS), "the fourth at once is refused");
    assert_eq!(b.take(10 * MS), Ok(()), "one token came back");
    assert_eq!(b.take(10 * MS), Err(10 * MS));
}

#[test]
fn bucket_never_banks_more_than_its_burst() {
    let mut b = bucket(10, 2);
    assert_eq!(b.take(0), Ok(()));
    let hour = 3_600_000 * MS;
    assert_eq!(b.take(hour), Ok(()));
    assert_eq!(b.take(hour), Ok(()));
    assert_eq!(
        b.take(hour),
        Err(10 * MS),
        "an idle hour still buys only the burst"
    );
}

#[test]
fn refusing_does_not_spend_a_token() {
    let mut b = bucket(10, 1);
    assert_eq!(b.take(0), Ok(()));
    for _ in 0..5 {
        assert!(b.take(MS).is_err());
    }
    assert_eq!(
        b.take(10 * MS),
        Ok(()),
        "refusals did not push the next token away"
    );
}

#[test]
fn extreme_buckets_do_not_overflow() {
    let mut huge = Bucket::new(&RateLimit {
        interval_ns: u64::MAX,
        burst: u32::MAX,
    });
    assert_eq!(huge.take(u64::MAX), Ok(()));
    assert_eq!(
        huge.take(u64::MAX),
        Ok(()),
        "a 4-billion burst is still open"
    );
    let mut tiny = Bucket::new(&RateLimit {
        interval_ns: 1,
        burst: 1,
    });
    for t in 0..1000 {
        assert_eq!(tiny.take(t), Ok(()), "1 token per ns at 1 GHz");
    }
    assert_eq!(tiny.take(999), Err(1));
}

#[test]
fn gate_runs_up_to_its_slots_then_queues_then_overflows() {
    let mut g = gate(1, Some(1));
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Queued);
    assert_eq!(g.arrive((2, 0)), Admit::Overflow);
    assert_eq!((g.busy(), g.queued(), g.slots()), (1, 1, 1));
    assert_eq!(g.overflow_status(), 503);
}

#[test]
fn a_queue_of_zero_overflows_as_soon_as_every_slot_is_busy() {
    let mut g = gate(2, Some(0));
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Run);
    assert_eq!(g.arrive((2, 0)), Admit::Overflow);
    assert_eq!(g.queued(), 0);
}

#[test]
fn release_hands_the_slot_to_the_oldest_waiter_without_freeing_it() {
    let mut g = gate(1, None);
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Queued);
    assert_eq!(g.arrive((2, 0)), Admit::Queued);
    assert_eq!(g.release(), Some((1, 0)), "FIFO");
    assert_eq!(g.busy(), 1, "the slot never looked free");
    assert_eq!(
        g.arrive((3, 0)),
        Admit::Queued,
        "a later arrival cannot barge"
    );
    assert_eq!(g.release(), Some((2, 0)));
    assert_eq!(g.release(), Some((3, 0)));
    assert_eq!(g.release(), None);
    assert_eq!(g.busy(), 0);
}

#[test]
fn an_unbounded_queue_never_overflows() {
    let mut g = gate(1, None);
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    for i in 1..10_000 {
        assert_eq!(g.arrive((i, 0)), Admit::Queued);
    }
    assert_eq!(g.queued(), 9_999);
}

#[test]
fn cancel_removes_one_waiter_and_keeps_the_order_of_the_rest() {
    let mut g = gate(1, None);
    g.arrive((0, 0));
    for i in 1..=3 {
        g.arrive((i, 0));
    }
    g.cancel((2, 0));
    assert_eq!(g.release(), Some((1, 0)));
    assert_eq!(g.release(), Some((3, 0)));
    g.cancel((9, 9)); // not queued: no effect
}

fn topo(yaml: &str) -> Topology {
    Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap()
}

fn hop(node: NodeId, kind: HopKind, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        kind,
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

fn plan(items: Vec<PlanItem>) -> Arc<Plan> {
    Arc::new(Plan { items })
}

/// Steps until nothing is scheduled and returns everything that finished.
fn run_all(e: &mut Engine) -> Vec<Finished> {
    while e.step().unwrap() {}
    e.drain_finished()
}

const ONE_SLOT: &str = "version: 1\nnodes:\n  app: { type: service, concurrency: 1 }\n";

fn ms(n: u64) -> u64 {
    n * MS
}

#[test]
fn fifo_hand_over_serves_waiters_in_arrival_order() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    for (i, at) in [0, 1, 2].into_iter().enumerate() {
        e.advance_to(ms(at)).unwrap();
        e.submit(ms(at), plan(vec![hop(app, HopKind::Proc, 10)]), i as u64)
            .unwrap();
    }
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.order);
    let waits: Vec<Duration> = done.iter().map(|f| f.hops[0].wait).collect();
    assert_eq!(
        waits,
        [0, 9, 18].map(Duration::from_millis),
        "10 ms hops through one slot, arriving at 0, 1 and 2 ms"
    );
    assert_eq!(
        done.iter().map(|f| f.done).collect::<Vec<_>>(),
        [ms(10), ms(20), ms(30)]
    );
}

#[test]
fn arrival_before_the_release_event_of_the_same_instant_queues_behind_waiters() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), 1)
        .unwrap();
    e.submit(ms(1), plan(vec![hop(app, HopKind::Proc, 10)]), 2)
        .unwrap();
    // tag 3 arrives at 10 ms, BEFORE the release event of tag 1 at 10 ms is stepped.
    e.submit(ms(10), plan(vec![hop(app, HopKind::Proc, 10)]), 3)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    // A hop's `start` is relative to its arrival; add the arrival for the absolute start.
    let starts: Vec<u64> = done
        .iter()
        .map(|f| f.arrival + u64::try_from(f.hops[0].hop.start.as_nanos()).unwrap())
        .collect();
    assert_eq!(
        starts,
        [0, ms(10), ms(20)],
        "tag 3 waits behind tag 2: no barging"
    );
}

#[test]
fn answer_is_emitted_at_the_failure_instant() {
    // Fork: branch 0 runs `slow` for 10 ms; branch 1 runs `wait` 5 ms, then fails at `bad`.
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  slow: { type: external-api }\n  wait: { type: external-api }\n  bad: { type: external-api }\n",
    );
    let (slow, wait, bad) = (
        t.id("slow").unwrap(),
        t.id("wait").unwrap(),
        t.id("bad").unwrap(),
    );
    let p = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 1 },
        hop(slow, HopKind::Proc, 10),
        PlanItem::Branch { size: 2 },
        hop(wait, HopKind::Proc, 5),
        PlanItem::Fail {
            node: bad,
            status: 503,
            at: Duration::from_millis(5),
        },
    ]);
    let mut e = Engine::new(&t);
    e.submit(0, p, 7).unwrap();
    e.advance_to(ms(5)).unwrap();
    let got = e.drain_finished();
    assert_eq!(
        got.len(),
        1,
        "answered at 5 ms although `slow` runs until 10 ms"
    );
    assert_eq!(got[0].done, ms(5));
    assert_eq!(
        got[0].result,
        ChainResult::Err {
            node: bad,
            status: 503,
            retry_after: None,
        }
    );
    let cut: Vec<(usize, Duration)> = got[0].hops.iter().map(|h| (h.idx, h.hop.dur)).collect();
    assert_eq!(
        cut,
        [(2, Duration::from_millis(5)), (4, Duration::from_millis(5))],
        "the running hop is cut at the failure"
    );
    assert_eq!(
        e.next_due(),
        Some(ms(10)),
        "the cancelled hop still holds its slot"
    );
    assert!(run_all(&mut e).is_empty(), "nothing is emitted twice");
    assert_eq!(e.table_len(), 1);
}

#[test]
fn stamp_orders_arrivals_with_events() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    let a = e.stamp();
    e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), 1)
        .unwrap();
    let (time, seq) = e.peek().unwrap();
    assert_eq!(time, ms(10));
    assert!(seq > a, "the event was scheduled after the stamp");
    assert!(e.stamp() > seq);
    assert_eq!(e.next_due(), Some(ms(10)));
}

#[test]
fn late_advance_gives_the_same_records_as_stepping_on_time() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: 1 }\n  db: { type: database, concurrency: 1, ops: { read: 3ms, write: 3ms, scan: 3ms } }\nedges:\n  - { from: app, to: db }\n",
    );
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let items = || {
        plan(vec![
            hop(app, HopKind::Proc, 4),
            hop(db, HopKind::Op(kaanal_core::Op::Read), 3),
        ])
    };
    let drive = |late: bool| {
        let mut e = Engine::new(&t);
        let mut out = Vec::new();
        for (i, at) in [0u64, 1, 2, 30].into_iter().enumerate() {
            if !late {
                e.advance_to(ms(at)).unwrap();
            }
            e.submit(ms(at), items(), i as u64).unwrap();
            if !late {
                out.extend(e.drain_finished());
            }
        }
        if late {
            e.advance_to(ms(3_600_000)).unwrap();
        } else {
            while e.step().unwrap() {}
        }
        out.extend(e.drain_finished());
        out.sort_by_key(|f| f.order);
        out
    };
    assert_eq!(
        drive(true),
        drive(false),
        "events run at their scheduled times, not at the call"
    );
}

#[test]
fn clock_overflow_is_an_error() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    let r = e.submit(u64::MAX - 5, plan(vec![hop(app, HopKind::Proc, 10)]), 1);
    assert_eq!(r, Err(EngineError::ClockOverflow));
    assert_eq!(
        e.step(),
        Err(EngineError::ClockOverflow),
        "the engine stays failed"
    );
}

#[test]
fn the_request_table_is_reused() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    for i in 0..100u64 {
        e.advance_to(ms(i * 20)).unwrap();
        e.submit(ms(i * 20), plan(vec![hop(app, HopKind::Proc, 10)]), i)
            .unwrap();
        e.advance_to(ms(i * 20 + 10)).unwrap();
    }
    assert_eq!(e.table_len(), 1, "sequential requests reuse one table slot");
    assert_eq!(e.drain_finished().len(), 100);
    assert_eq!(e.node_counters()[app].completed, 100);
}

#[test]
fn submit_runs_earlier_events_itself_and_keeps_same_instant_ties() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), 1)
        .unwrap();
    e.submit(ms(1), plan(vec![hop(app, HopKind::Proc, 10)]), 2)
        .unwrap();
    // No `advance_to`: submit runs the 10 ms release first, handing the slot to tag 2.
    e.submit(ms(15), plan(vec![hop(app, HopKind::Proc, 10)]), 3)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(9));
    assert_eq!(
        done[2].hops[0].wait,
        Duration::from_millis(5),
        "tag 3 queued behind tag 2, which ends at 20 ms"
    );
    assert_eq!(done[2].done, ms(30));
}

#[test]
#[should_panic(expected = "earlier than the engine clock")]
fn submitting_into_the_past_is_a_clear_panic() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(ms(10), plan(vec![hop(app, HopKind::Proc, 1)]), 1)
        .unwrap();
    e.submit(ms(5), plan(vec![hop(app, HopKind::Proc, 1)]), 2)
        .unwrap();
}

#[test]
fn drain_finished_iter_yields_the_same_answers_and_empties_the_buffer() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let drive = || {
        let mut e = Engine::new(&t);
        for i in 0..3u64 {
            e.submit(ms(i), plan(vec![hop(app, HopKind::Proc, 10)]), i)
                .unwrap();
        }
        while e.step().unwrap() {}
        e
    };
    let want = drive().drain_finished();
    let mut e = drive();
    let got: Vec<Finished> = e.drain_finished_iter().collect();
    assert_eq!(got, want);
    assert_eq!(got.len(), 3);
    assert!(e.drain_finished().is_empty(), "drained once");
}

const LIMITED: &str = "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1, burst: 1 }, concurrency: 1 }\n";

fn err_of(f: &Finished) -> (NodeId, u16, Option<Duration>) {
    match f.result {
        ChainResult::Err {
            node,
            status,
            retry_after,
        } => (node, status, retry_after),
        ChainResult::Ok => panic!("expected a failure: {f:?}"),
    }
}

#[test]
fn an_empty_bucket_fails_the_request_with_429_and_retry_after_and_takes_no_slot() {
    let t = topo(LIMITED);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(gw, HopKind::Proc, 10)]), 1)
        .unwrap();
    e.advance_to(ms(1)).unwrap();
    e.submit(ms(1), plan(vec![hop(gw, HopKind::Proc, 10)]), 2)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(
        err_of(&done[1]),
        (gw, 429, Some(Duration::from_millis(999)))
    );
    assert_eq!(done[1].done, ms(1), "refused at once");
    assert!(done[1].hops.is_empty(), "the refused hop never started");
    let c = &e.node_counters()[gw];
    assert_eq!(
        (c.limited, c.completed, c.max_queue),
        (1, 1, 0),
        "it did not queue for the slot either"
    );
}

#[test]
fn a_full_queue_fails_with_the_gate_status() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, flavor: postgres, pool: { size: 1, queue: 1, overflow_status: 504 } }\nedges:\n  - { from: app, to: db }\n",
    );
    let db = t.id("db").unwrap();
    let read = HopKind::Op(kaanal_core::Op::Read);
    let mut e = Engine::new(&t);
    for tag in 0..3 {
        e.submit(0, plan(vec![hop(db, read, 10)]), tag).unwrap();
    }
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(done[1].result, ChainResult::Ok);
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(10));
    assert_eq!(err_of(&done[2]), (db, 504, None));
    assert_eq!(done[2].done, 0);
    assert_eq!(e.node_counters()[db].overflowed, 1);
    assert_eq!(e.node_counters()[db].max_queue, 1);
}

#[test]
fn release_instant_arrival_does_not_barge_with_a_bounded_queue() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, flavor: postgres, pool: { size: 1, queue: 1 } }\nedges:\n  - { from: app, to: db }\n",
    );
    let db = t.id("db").unwrap();
    let read = HopKind::Op(kaanal_core::Op::Read);
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(db, read, 10)]), 1).unwrap();
    e.submit(0, plan(vec![hop(db, read, 10)]), 2).unwrap(); // waits
    e.submit(ms(10), plan(vec![hop(db, read, 10)]), 3).unwrap(); // before the release event: queue is full
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(
        err_of(&done[2]).1,
        503,
        "the queue still held tag 2 when tag 3 arrived"
    );
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(10));
}

#[test]
fn a_visit_takes_one_token_however_many_own_hops_it_has() {
    let t = topo(
        "version: 1\nnodes:\n  m: { type: monolith, flavor: postgres, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let m = t.id("m").unwrap();
    let mut e = Engine::new(&t);
    // proc then the inline op: one visit, one token.
    e.submit(
        0,
        plan(vec![
            hop(m, HopKind::Proc, 1),
            hop(m, HopKind::Op(kaanal_core::Op::Read), 1),
        ]),
        1,
    )
    .unwrap();
    let done = run_all(&mut e);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(e.node_counters()[m].limited, 0);
}

#[test]
fn each_call_of_a_node_is_its_own_visit() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 2 } }\nedges:\n  - { from: app, to: s }\n",
    );
    let s = t.id("s").unwrap();
    let call = || [hop(s, HopKind::Net, 1), hop(s, HopKind::Proc, 1)];
    let mut e = Engine::new(&t);
    // repeat: 3 → net, proc, net, proc, net, proc: three visits, a burst of 2 refuses the third.
    let items: Vec<PlanItem> = (0..3).flat_map(|_| call()).collect();
    e.submit(0, plan(items), 1).unwrap();
    let done = run_all(&mut e);
    let (node, status, _) = err_of(&done[0]);
    assert_eq!((node, status), (s, 429));
    assert_eq!(e.node_counters()[s].limited, 1);
    assert_eq!(
        done[0].hops.len(),
        5,
        "two full visits and the third visit's net hop"
    );
}

#[test]
fn parallel_branches_each_take_a_token() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\nedges:\n  - { from: app, to: s }\n",
    );
    let s = t.id("s").unwrap();
    let p = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 6,
        },
        PlanItem::Branch { size: 2 },
        hop(s, HopKind::Net, 1),
        hop(s, HopKind::Proc, 5),
        PlanItem::Branch { size: 2 },
        hop(s, HopKind::Net, 1),
        hop(s, HopKind::Proc, 5),
    ]);
    let mut e = Engine::new(&t);
    e.submit(0, p, 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        err_of(&done[0]).1,
        429,
        "the second branch's visit finds the bucket empty"
    );
}

#[test]
fn a_chaos_failure_takes_no_token() {
    let t = topo("version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n");
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![PlanItem::Fail {
            node: gw,
            status: 503,
            at: Duration::ZERO,
        }]),
        1,
    )
    .unwrap();
    e.submit(0, plan(vec![hop(gw, HopKind::Proc, 1)]), 2)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[0]), (gw, 503, None));
    assert_eq!(
        done[1].result,
        ChainResult::Ok,
        "the planned failure spent no token"
    );
}

#[test]
fn an_engine_failure_cancels_queued_siblings_like_a_planned_one() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, concurrency: 1 }\n  lim: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let (s, lim) = (t.id("s").unwrap(), t.id("lim").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(s, HopKind::Proc, 10)]), 1)
        .unwrap(); // holds `s`
    e.submit(0, plan(vec![hop(lim, HopKind::Proc, 1)]), 2)
        .unwrap(); // empties the bucket
    let fork = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(s, HopKind::Proc, 10), // queues behind tag 1
        PlanItem::Branch { size: 1 },
        hop(lim, HopKind::Proc, 1), // refused
    ]);
    e.submit(0, fork, 3).unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[2]).1, 429);
    assert_eq!(done[2].hops, []);
    assert_eq!(
        e.node_counters()[s].completed,
        1,
        "the queued sibling never ran"
    );
}

#[test]
fn a_planned_failure_that_replaces_a_refusal_takes_its_count_back() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  free: { type: external-api }\n  g1: { type: external-api }\n  b3: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\nedges:\n  - { from: app, to: free }\n  - { from: app, to: g1 }\n  - { from: app, to: b3 }\n",
    );
    let (app, free, g1, b3) = (
        t.id("app").unwrap(),
        t.id("free").unwrap(),
        t.id("g1").unwrap(),
        t.id("b3").unwrap(),
    );
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(b3, HopKind::Proc, 1)]), 1)
        .unwrap(); // spends b3's only token
    // Branch 0 reaches its planned Fail at 3 ms (low plan index); branch 1 reaches b3 at 3 ms and is
    // refused first (its Net hop was scheduled earlier). D17: the lower index still wins.
    let p = plan(vec![
        hop(app, HopKind::Proc, 0),
        PlanItem::Fork {
            branches: 2,
            size: 8,
        },
        PlanItem::Branch { size: 4 },
        hop(free, HopKind::Net, 1),
        hop(free, HopKind::Proc, 1),
        hop(g1, HopKind::Net, 1),
        PlanItem::Fail {
            node: g1,
            status: 500,
            at: Duration::from_millis(3),
        },
        PlanItem::Branch { size: 2 },
        hop(b3, HopKind::Net, 3),
        hop(b3, HopKind::Proc, 1),
    ]);
    e.submit(0, p, 2).unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[1]), (g1, 500, None));
    assert_eq!(done[1].done, ms(3));
    assert_eq!(
        e.node_counters()[b3].limited,
        0,
        "the refusal did not decide the answer, so it is not counted"
    );
}

#[test]
fn an_own_hop_after_a_join_starts_a_new_visit() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\n  x: { type: external-api }\nedges:\n  - { from: app, to: s }\n  - { from: app, to: x }\n",
    );
    let (lim, other) = (t.id("s").unwrap(), t.id("x").unwrap());
    let mut e = Engine::new(&t);
    // s's visit, then a parallel block whose branch calls x over an edge, then s again: a new visit.
    let p = plan(vec![
        hop(lim, HopKind::Proc, 1),
        PlanItem::Fork {
            branches: 1,
            size: 2,
        },
        PlanItem::Branch { size: 1 },
        hop(other, HopKind::Net, 1),
        hop(lim, HopKind::Proc, 1),
    ]);
    e.submit(0, p, 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        err_of(&done[0]).1,
        429,
        "the second visit finds the bucket empty"
    );
}
