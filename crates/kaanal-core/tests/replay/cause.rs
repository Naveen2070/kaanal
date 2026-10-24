//! The failure cause reported for each way a request can fail.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{ChainResult, FailKind, HopKind, NodeId, Op, PlanItem};
use std::time::Duration;

pub(crate) fn planned_fail(node: NodeId, kind: FailKind, status: u16) -> PlanItem {
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
