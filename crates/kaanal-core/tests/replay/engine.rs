//! The engine: FIFO hand-over, answers, stamps, the clock, the request table, submit and drain.

use crate::common::*;
use kaanal_core::replay::{Engine, EngineError, Finished};
use kaanal_core::{ChainResult, FailKind, HopKind, PlanItem};
use std::time::Duration;

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
