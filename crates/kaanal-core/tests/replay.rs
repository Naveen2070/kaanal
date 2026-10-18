//! The replay engine and its parts: `Bucket` and `Gate` (Task 2), `Engine` (Tasks 3 and 4).

use kaanal_core::replay::{Admit, Bucket, Engine, EngineError, FailCause, Finished, Gate};
use kaanal_core::topology::{Admission, RateLimit};
use kaanal_core::{
    ChainResult, Config, FailKind, Hop, HopKind, NodeId, Op, Plan, PlanItem, Topology,
};
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
            kind: FailKind::Status,
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
            kind: FailKind::Status,
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
            kind: _,
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
            kind: FailKind::Status,
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
            kind: FailKind::Status,
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
    assert_eq!(
        done[1].cause,
        Some(FailCause::Chaos),
        "the replaced rate-limit refusal leaves no RateLimit cause"
    );
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

const POOL1: &str = "version: 1\nnodes:\n  gw: { type: service }\n  db: { type: database, ops: { read: 400ms, write: 400ms, scan: 400ms }, pool: { size: 1, queue: 5 } }\nedges:\n  - { from: gw, to: db, net: 1ms }\n";

fn span(node: NodeId, ms: u64, size: usize) -> PlanItem {
    PlanItem::Span {
        node,
        timeout: Duration::from_millis(ms),
        at: Duration::ZERO,
        size,
    }
}

fn fin(f: &[Finished], order: u64) -> &Finished {
    f.iter().find(|x| x.order == order).unwrap()
}

#[test]
fn zombie_hop_keeps_the_slot_and_waiters_time_out_in_the_queue() {
    // spec §1 criterion 2
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mk = || plan(vec![span(db, 500, 1), hop(db, HopKind::Op(Op::Read), 400)]);
    let mut e = Engine::new(&t);
    for _ in 0..3 {
        e.submit(0, mk(), 0).unwrap();
    }
    e.submit(600 * MS, mk(), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f.len(), 4);
    assert_eq!(fin(&f, 0).result, ChainResult::Ok);
    assert_eq!(fin(&f, 0).done, 400 * MS);
    // request 1: got the slot at 400, times out while running at 500, hop is a zombie until 800
    let r1 = fin(&f, 1);
    assert_eq!(r1.done, 500 * MS);
    assert!(matches!(
        r1.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
    assert_eq!(
        r1.hops[0].hop.dur,
        Duration::from_millis(100),
        "client view is cut at the deadline"
    );
    // request 2: leaves the queue at 500
    let r2 = fin(&f, 2);
    assert_eq!(r2.done, 500 * MS);
    assert_eq!(r2.hops.len(), 0);
    // request 3 arrives at 600, waits for the zombie until 800, deadline at 1100 hits it mid-run
    let r3 = fin(&f, 3);
    assert_eq!(r3.done, 1100 * MS);
    assert!(matches!(
        r3.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            ..
        }
    ));
    assert_eq!(r3.hops[0].wait, Duration::from_millis(200));
    let c = &e.node_counters()[db];
    assert_eq!(c.timed_out, 3);
    // r0, r1 and r3 started a hop; r2 left the queue unserved
    assert_eq!(c.completed, 3);
    // waits: r0 0, r1 400, r3 200
    assert_eq!(c.wait, u128::from(600 * MS));
}

#[test]
fn finishing_exactly_at_the_deadline_is_ok() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(db, 400, 1), hop(db, HopKind::Op(Op::Read), 400)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].result, ChainResult::Ok);
    assert_eq!(f[0].done, 400 * MS);
}

#[test]
fn a_planned_fail_at_the_deadline_instant_beats_the_deadline() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(db, 400, 2),
            hop(db, HopKind::Op(Op::Read), 400),
            PlanItem::Fail {
                node: db,
                status: 500,
                kind: FailKind::Status,
                at: Duration::from_millis(400),
            },
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert!(
        matches!(
            f[0].result,
            ChainResult::Err {
                status: 500,
                kind: FailKind::Status,
                ..
            }
        ),
        "{:?}",
        f[0].result
    );
}

#[test]
fn span_ending_in_a_parallel_block_stays_open() {
    // review B3: the fork moves the parent token to the block end at once; the span must not close there
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let items = vec![
        span(gw, 300, 5),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
    ];
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].done, 300 * MS);
    assert!(matches!(
        f[0].result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            ..
        }
    ));
}

#[test]
fn stale_deadline_never_hits_a_recycled_request_slot() {
    // review B2: A finishes at 100 ms, its table slot is reused by B at 150 ms,
    // A's deadline (300 ms) pops while B runs and must be ignored.
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(gw, 300, 1), hop(gw, HopKind::Proc, 100)]),
        0,
    )
    .unwrap();
    e.advance_to(150 * MS).unwrap();
    // B has its own span (a different deadline seq), so a stale deadline cannot match it by accident
    e.submit(
        150 * MS,
        plan(vec![span(gw, 5000, 1), hop(gw, HopKind::Proc, 1000)]),
        1,
    )
    .unwrap();
    let f = run_all(&mut e);
    let b = f.iter().find(|x| x.tag == 1).unwrap();
    assert_eq!(b.result, ChainResult::Ok);
    assert_eq!(b.done, 1150 * MS);
    assert_eq!(e.table_len(), 1, "B reused A's slot");
}

#[test]
fn closed_span_ignores_its_deadline() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(gw, 300, 1),
            hop(gw, HopKind::Proc, 100),
            hop(gw, HopKind::Proc, 500),
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(
        f[0].result,
        ChainResult::Ok,
        "the span ended at 100 ms; the later hop is outside it"
    );
    assert_eq!(f[0].done, 600 * MS);
}

#[test]
fn nested_spans_outer_wins_at_equal_deadlines() {
    let t = topo(POOL1);
    let (gw, db) = (t.id("gw").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(gw, 200, 2),
            span(db, 200, 1),
            hop(db, HopKind::Op(Op::Read), 400),
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    let ChainResult::Err { node, .. } = f[0].result else {
        panic!()
    };
    assert_eq!(
        node, gw,
        "equal deadlines: the open span with the lowest plan index (the outer one) fires (Z14)"
    );
}

#[test]
fn deadline_inside_a_parallel_branch_cancels_the_siblings() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let items = vec![
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 2 },
        span(gw, 100, 1),
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
    ];
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].done, 100 * MS);
    assert_eq!(f[0].cause, Some(FailCause::Deadline));
    assert!(
        matches!(
            f[0].result,
            ChainResult::Err { node, kind: FailKind::Timeout, status: 504, .. } if node == gw
        ),
        "{:?}",
        f[0].result
    );
    assert_eq!(f[0].hops.len(), 2);
    assert!(
        f[0].hops
            .iter()
            .all(|h| h.hop.dur <= Duration::from_millis(100)),
        "sibling cut at the failure"
    );
}

const THREE: &str =
    "version: 1\nnodes:\n  a: { type: service }\n  b: { type: service }\n  c: { type: service }\n";

fn timeout_node(f: &Finished) -> NodeId {
    match f.result {
        ChainResult::Err {
            node,
            kind: FailKind::Timeout,
            ..
        } => node,
        ref r => panic!("expected a timeout, got {r:?}"),
    }
}

#[test]
fn equal_deadlines_in_two_branches_fail_at_the_lower_plan_index() {
    // Z14 / `execute`: the lower plan index wins, however the spans were opened in time.
    let t = topo(THREE);
    let [na, nb, nc] = ["a", "b", "c"].map(|n| t.id(n).unwrap());
    // b (index 3) opens at 50 ms, c (index 6) at 0: c is stamped first, both fall at 100 ms
    let c_first = vec![
        PlanItem::Fork {
            branches: 2,
            size: 7,
        },
        PlanItem::Branch { size: 3 },
        hop(na, HopKind::Proc, 50),
        span(nb, 50, 1),
        hop(nb, HopKind::Proc, 200),
        PlanItem::Branch { size: 2 },
        span(nc, 100, 1),
        hop(nc, HopKind::Proc, 200),
    ];
    // b (index 2) opens at 0 and is stamped first; c (index 6) opens at 50
    let b_first = vec![
        PlanItem::Fork {
            branches: 2,
            size: 7,
        },
        PlanItem::Branch { size: 2 },
        span(nb, 100, 1),
        hop(nb, HopKind::Proc, 200),
        PlanItem::Branch { size: 3 },
        hop(na, HopKind::Proc, 50),
        span(nc, 50, 1),
        hop(nc, HopKind::Proc, 200),
    ];
    for items in [c_first, b_first] {
        let mut e = Engine::new(&t);
        e.submit(0, plan(items), 0).unwrap();
        let f = run_all(&mut e);
        assert_eq!(f[0].done, 100 * MS);
        assert_eq!(timeout_node(&f[0]), nb);
        assert_eq!(e.node_counters()[nb].timed_out, 1);
        assert_eq!(e.node_counters()[nc].timed_out, 0);
    }
}

#[test]
fn a_deadline_at_the_hand_over_instant_leaves_a_zombie_that_the_client_never_sees() {
    // r1 waits exactly its 400 ms: r0's hop-done (a normal event) runs first and hands r1 the
    // slot, then the deadline fires. The hop started at the failure instant, so it is absent.
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let read = || hop(db, HopKind::Op(Op::Read), 400);
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![span(db, 2000, 1), read()]), 0)
        .unwrap();
    e.submit(0, plan(vec![span(db, 400, 1), read()]), 1)
        .unwrap();
    e.submit(0, plan(vec![span(db, 2000, 1), read()]), 2)
        .unwrap();
    let f = run_all(&mut e);
    let r1 = fin(&f, 1);
    assert_eq!(timeout_node(r1), db);
    assert_eq!(r1.done, 400 * MS);
    assert_eq!(r1.hops.len(), 0, "started at the failure instant: absent");
    // the zombie held the slot until 800, so r2 starts then
    let r2 = fin(&f, 2);
    assert_eq!(r2.result, ChainResult::Ok);
    assert_eq!(r2.done, 1200 * MS);
    assert_eq!(r2.hops[0].wait, Duration::from_millis(800));
    assert_eq!(e.node_counters()[db].completed, 3);
    assert_eq!(e.node_counters()[db].timed_out, 1);
}

#[test]
fn a_deadline_over_a_delay_hop_cuts_it() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(gw, 100, 1), hop(gw, HopKind::Delay, 500)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(timeout_node(&f[0]), gw);
    assert_eq!(f[0].done, 100 * MS);
    assert_eq!(f[0].hops[0].hop.dur, Duration::from_millis(100));
    assert_eq!(e.node_counters()[gw].timed_out, 1);
}

#[test]
fn a_deadline_with_one_sibling_queued_and_another_running() {
    let t = topo(POOL1);
    let (gw, db) = (t.id("gw").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    // a blocker holds the only db slot until 400 ms
    e.submit(0, plan(vec![hop(db, HopKind::Op(Op::Read), 400)]), 0)
        .unwrap();
    let items = vec![
        span(gw, 200, 5),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(db, HopKind::Op(Op::Read), 400),
    ];
    e.submit(0, plan(items), 1).unwrap();
    let f = run_all(&mut e);
    let r = fin(&f, 1);
    assert_eq!(timeout_node(r), gw);
    assert_eq!(r.done, 200 * MS);
    assert_eq!(r.hops.len(), 1, "the queued sibling never started");
    assert_eq!(r.hops[0].hop.dur, Duration::from_millis(200));
    // the cancelled waiter did not take the slot when the blocker released it
    assert_eq!(e.node_counters()[db].completed, 1);
    assert_eq!(fin(&f, 0).result, ChainResult::Ok);
}

#[test]
fn timed_out_is_not_counted_for_a_pre_warmup_arrival() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.set_window(100 * MS, u64::MAX);
    let mk = || plan(vec![span(db, 50, 1), hop(db, HopKind::Op(Op::Read), 400)]);
    e.submit(0, mk(), 0).unwrap();
    e.submit(500 * MS, mk(), 1).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f.len(), 1, "the pre-warmup request is not reported");
    assert_eq!(f[0].tag, 1);
    assert_eq!(timeout_node(&f[0]), db);
    assert_eq!(e.node_counters()[db].timed_out, 1);
}

#[test]
fn delay_hop_takes_no_slot_and_no_token() {
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 1, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    // two requests at once: each is one 50 ms delay (holds nothing)
    let mk = || plan(vec![hop(gw, HopKind::Delay, 50)]);
    e.submit(0, mk(), 0).unwrap();
    e.submit(0, mk(), 1).unwrap();
    let f = run_all(&mut e);
    assert!(
        f.iter().all(|x| x.result == ChainResult::Ok),
        "no token needed, no slot, no 429"
    );
    assert!(f.iter().all(|x| x.done == 50 * MS));
}

/// Independent reference for one FIFO server, unbounded queue, patience `d` on the wait:
/// a customer whose wait would exceed `d` leaves; one that starts always runs to the end (zombie).
/// Client-visible success needs wait + service <= d. Per customer: (0 ok / 1 reneged in the queue /
/// 2 overran in service, ns from arrival to the client's answer).
fn reference(arrivals: &[(u64, u64)], d: u64) -> Vec<(u8, u64)> {
    let mut free = 0u64;
    arrivals
        .iter()
        .map(|&(t, s)| {
            let w = free.saturating_sub(t);
            if w > d {
                return (1, d);
            }
            free = t + w + s;
            if w + s <= d { (0, w + s) } else { (2, d) }
        })
        .collect()
}

#[test]
fn engine_matches_the_reference_queue_with_patience() {
    use rand::{Rng, SeedableRng};
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms }, pool: { size: 1, queue: 100000 } }\n",
    );
    let db = t.id("db").unwrap();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let (d, mut clock) = (80 * MS, 0u64);
    let arrivals: Vec<(u64, u64)> = (0..4000)
        .map(|_| {
            clock += rng.random_range(0..20 * MS); // mean gap ~10 ms
            (clock, rng.random_range(MS..19 * MS)) // service 1..19 ms, rho ~1: both reneging and overruns occur
        })
        .collect();
    let mut e = Engine::new(&t);
    for (i, &(at, s)) in arrivals.iter().enumerate() {
        let p = plan(vec![
            span(db, d / MS, 1),
            PlanItem::Hop(Hop {
                node: db,
                kind: HopKind::Op(Op::Read),
                start: Duration::ZERO,
                dur: Duration::from_nanos(s),
            }),
        ]);
        e.submit(at, p, i as u64).unwrap();
    }
    let f = {
        let mut v = run_all(&mut e);
        v.sort_by_key(|x| x.tag);
        v
    };
    let want = reference(&arrivals, d);
    for (i, &(outcome, answered)) in want.iter().enumerate() {
        assert_eq!(f[i].result == ChainResult::Ok, outcome == 0, "request {i}");
        assert_eq!(
            f[i].done,
            arrivals[i].0 + answered,
            "request {i} answer time"
        );
    }
    // the run must exercise both ways to time out, or the comparison proves little
    let reneged = want.iter().filter(|w| w.0 == 1).count();
    let overran = want.iter().filter(|w| w.0 == 2).count();
    assert!(
        reneged > 50 && overran > 50,
        "reneged {reneged}, overran {overran}"
    );
}

fn planned_fail(node: NodeId, kind: FailKind, status: u16) -> PlanItem {
    PlanItem::Fail {
        node,
        status,
        kind,
        at: Duration::ZERO,
    }
}

#[test]
fn cause_is_none_on_success() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), 0)
        .unwrap();
    let f = run_all(&mut e);
    assert_eq!((&f[0].result, f[0].cause), (&ChainResult::Ok, None));
}

#[test]
fn cause_chaos_for_a_planned_failure_and_a_hang() {
    let t = topo(ONE_SLOT);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![planned_fail(app, FailKind::Status, 503)]), 0)
        .unwrap();
    e.submit(0, plan(vec![planned_fail(app, FailKind::Timeout, 504)]), 1)
        .unwrap();
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 0).cause, Some(FailCause::Chaos));
    assert_eq!(fin(&f, 1).cause, Some(FailCause::Chaos), "a hang is chaos");
}

#[test]
fn cause_rate_limit_for_a_429_from_the_bucket() {
    let t = topo(LIMITED);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    for tag in 0..2 {
        e.submit(0, plan(vec![hop(gw, HopKind::Proc, 10)]), tag)
            .unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 0).cause, None);
    assert_eq!(fin(&f, 1).cause, Some(FailCause::RateLimit));
}

#[test]
fn cause_overflow_for_a_full_queue() {
    let t = topo(
        "version: 1
nodes:
  app: { type: service }
  db: { type: database, flavor: postgres, pool: { size: 1, queue: 0 } }
edges:
  - { from: app, to: db }
",
    );
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    for tag in 0..2 {
        e.submit(0, plan(vec![hop(db, HopKind::Op(Op::Read), 10)]), tag)
            .unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 1).cause, Some(FailCause::Overflow));
}

#[test]
fn cause_deadline_for_a_fired_timeout() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(db, 100, 1), hop(db, HopKind::Op(Op::Read), 400)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].cause, Some(FailCause::Deadline));
}

#[test]
fn cause_does_not_change_any_existing_outcome() {
    // The fixtures of `an_empty_bucket_...` and `a_full_queue_...`, same expectations.
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
    assert_eq!(
        (done[0].cause, done[1].cause),
        (None, Some(FailCause::RateLimit))
    );
    assert_eq!((done[0].done, done[1].done), (ms(10), ms(1)));
    assert_eq!((done[0].hops.len(), done[1].hops.len()), (1, 0));

    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(db, 100, 1), hop(db, HopKind::Op(Op::Read), 400)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert!(matches!(
        f[0].result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
    assert_eq!(f[0].cause, Some(FailCause::Deadline));
    assert_eq!(f[0].done, ms(100));
    assert_eq!(f[0].hops[0].hop.dur, Duration::from_millis(100));
}
