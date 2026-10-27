//! The engine against a reference queue model.

use crate::common::*;
use kaanal_core::replay::Engine;
use kaanal_core::{ChainResult, Hop, HopKind, Op, PlanItem};
use std::time::Duration;

/// Independent reference for one FIFO server, unbounded queue, patience `d` on the wait:
/// a customer whose wait would exceed `d` leaves; one that starts always runs to the end (zombie).
/// Client-visible success needs wait + service <= d. Per customer: (0 ok / 1 reneged in the queue /
/// 2 overran in service, ns from arrival to the client's answer).
pub(crate) fn reference(arrivals: &[(u64, u64)], d: u64) -> Vec<(u8, u64)> {
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
                replica: 0,
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
