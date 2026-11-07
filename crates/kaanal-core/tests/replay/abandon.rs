//! `abandon: continue` (RULES 88, step 7b): when a caller's deadline passes, the work of the call it
//! gave up on goes on and loads the callee, while the caller is answered (or retries) as before.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{ChainResult, FailKind, HopKind, NodeId, Op, PlanItem};
use std::time::Duration;

const TWO: &str = "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, ops: { read: 5ms, write: 5ms, scan: 5ms } }\nedges:\n  - { from: app, to: db }\n";

fn deadline(node: NodeId, ms: u64, size: usize, orphan: bool) -> PlanItem {
    PlanItem::Span {
        node,
        timeout: Duration::from_millis(ms),
        at: Duration::ZERO,
        size,
        orphan,
    }
}

/// A call with a 10 ms deadline whose work is 50 ms at the app, then 5 ms at the db.
fn call(app: NodeId, db: NodeId, orphan: bool) -> std::sync::Arc<kaanal_core::Plan> {
    plan(vec![
        deadline(app, 10, 2, orphan),
        hop(app, HopKind::Proc, 50),
        hop(db, HopKind::Op(Op::Read), 5),
    ])
}

fn run_one(orphan: bool) -> (kaanal_core::replay::Finished, u64) {
    let t = topo(TWO);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, call(app, db, orphan), 0).unwrap();
    let f = run_all(&mut e);
    (f[0].clone(), e.node_counters()[db].completed)
}

#[test]
fn abandoned_work_goes_on_but_the_callers_answer_is_the_same() {
    let (cancel, cancel_db) = run_one(false);
    let (cont, cont_db) = run_one(true);
    // the caller is answered by its deadline either way, with the same record
    assert_eq!(cont, cancel, "the client's view does not change");
    assert!(matches!(
        cont.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
    assert_eq!((cont.done, cont.cause), (ms(10), Some(FailCause::Deadline)));
    // ...but the callee still does the work the caller left behind
    assert_eq!(cancel_db, 0, "cancel: the db hop never starts");
    assert_eq!(
        cont_db, 1,
        "continue: it starts when the app finishes and runs to its end"
    );
}

#[test]
fn load_at_the_callee_with_timeouts_equals_load_without_them() {
    // Little's law counts every unit of work the callee does: with `continue` the abandoned requests
    // still arrive at the db, so its load (and busy time) equals that of the same requests with no
    // deadline at all; with `cancel` the timed-out ones never reach it.
    let t = topo(TWO);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let run = |plans: Vec<std::sync::Arc<kaanal_core::Plan>>| {
        let mut e = Engine::new(&t);
        for (i, p) in plans.into_iter().enumerate() {
            e.submit(i as u64 * 1000 * MS, p, i as u64).unwrap();
        }
        let _ = run_all(&mut e);
        e.node_counters()[db].clone()
    };
    let n = 20;
    let none = run((0..n)
        .map(|_| {
            plan(vec![
                hop(app, HopKind::Proc, 50),
                hop(db, HopKind::Op(Op::Read), 5),
            ])
        })
        .collect());
    let cont = run((0..n).map(|_| call(app, db, true)).collect());
    let cancel = run((0..n).map(|_| call(app, db, false)).collect());
    assert_eq!(cont.completed, none.completed);
    assert_eq!(cont.area, none.area, "same busy time at the callee");
    assert_eq!(cancel.completed, 0);
}

#[test]
fn abandoned_work_after_a_join_still_runs() {
    // inside the call: two branches (30 ms and 20 ms), then the db: the request times out at 10 ms,
    // the block ends at 30 ms and the db hop after it still runs
    let t = topo(TWO);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let items = vec![
        deadline(app, 10, 6, true),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(app, HopKind::Proc, 30),
        PlanItem::Branch { size: 1 },
        hop(app, HopKind::Proc, 20),
        hop(db, HopKind::Op(Op::Read), 5),
    ];
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].done, ms(10));
    assert_eq!(e.node_counters()[db].completed, 1);
}

#[test]
fn an_abandoned_attempt_goes_on_while_the_retry_runs() {
    // attempt 0 times out at 10 ms (its work would take 55); the retry starts 5 ms later and is
    // done at 18. With `continue` attempt 0's db hop still runs at 50 ms; the request is unchanged.
    let t = topo(TWO);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let go = |orphan: bool| {
        let mut items = vec![
            PlanItem::Retry {
                attempts: 2,
                size: 0,
                budget: None,
                breaker: None,
                edge: 0,
            },
            PlanItem::Attempt {
                backoff: Duration::ZERO,
                size: 3,
            },
            deadline(app, 10, 2, orphan),
            hop(app, HopKind::Proc, 50),
            hop(db, HopKind::Op(Op::Read), 5),
            PlanItem::Attempt {
                backoff: Duration::from_millis(5),
                size: 2,
            },
            deadline(app, 10, 1, orphan),
            hop(app, HopKind::Proc, 3),
        ];
        items[0] = PlanItem::Retry {
            attempts: 2,
            size: items.len() - 1,
            budget: None,
            breaker: None,
            edge: 0,
        };
        let mut e = Engine::new(&t);
        e.submit(0, plan(items), 0).unwrap();
        let f = run_all(&mut e);
        (f[0].clone(), e.node_counters()[db].completed)
    };
    let (cancel, cancel_db) = go(false);
    let (cont, cont_db) = go(true);
    assert_eq!(cont, cancel, "the request is answered the same way");
    assert_eq!((cont.result, cont.done), (ChainResult::Ok, ms(18)));
    assert_eq!((cancel_db, cont_db), (0, 1));
}
