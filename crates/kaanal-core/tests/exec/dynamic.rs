//! `least-conn` and `p2c` in the walk: the plan leaves the choice to the engine (a `Pick` item and
//! a code on the hops) and `execute` settles it for zero load.

use crate::common::*;

const KEY: &str = "GET /users/{id}";

fn cfg(db_fields: &str) -> String {
    format!(
        "version: 1
nodes:
  nginx: {{ type: gateway, proc: 1ms }}
  app: {{ type: service, proc: 5ms }}
  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }}{db_fields} }}
edges:
  - {{ from: nginx, to: app, net: 1ms }}
  - {{ from: app, to: db, net: 1ms }}
endpoints:
  \"{KEY}\": {{ chain: [ db ] }}
"
    )
}

#[test]
fn least_conn_and_p2c_leave_the_choice_to_the_engine_but_settle_it_for_zero_load() {
    use kaanal_core::{DYN_BASE, PickKind, PlanItem, sample_plan};
    for (policy, kind, cands) in [
        ("least-conn", PickKind::LeastConn, 3),
        ("p2c", PickKind::P2c, 2),
    ] {
        let (t, e) = build(&cfg(&format!(", replicas: 3, balance: {policy}")));
        let db = t.id("db").unwrap();
        let ep = e.get(KEY).unwrap();
        let req = kaanal_core::ChainRequest {
            endpoint: ep,
            n: 5,
            seed: 42,
            pin_delay: std::time::Duration::ZERO,
            req_bytes: 0,
            resp_bytes: 0,
        };
        let (plan, _) = sample_plan(&t, &kaanal_core::ChaosState::default().compile(), &req);
        let picks: Vec<_> = plan
            .items
            .iter()
            .filter_map(|i| match i {
                PlanItem::Pick {
                    node,
                    kind: k,
                    cands: c,
                    ..
                } => Some((*node, *k, c.count_ones())),
                _ => None,
            })
            .collect();
        assert_eq!(picks, [(db, kind, cands)], "{policy}");
        // every hop of the visit carries the code, none a real copy
        let reps: Vec<u16> = plan
            .items
            .iter()
            .filter_map(|i| match i {
                PlanItem::Hop(h) if h.node == db => Some(h.replica),
                _ => None,
            })
            .collect();
        assert!(
            !reps.is_empty() && reps.iter().all(|r| *r >= DYN_BASE),
            "{policy}: {reps:?}"
        );
        // `execute` (zero load) settles it: a real copy in every hop
        let out = run(&t, &e, &kaanal_core::ChaosState::default(), 5);
        assert!(out.hops.iter().all(|h| h.replica < 3), "{policy}");
    }
}

#[test]
fn a_down_copy_is_not_a_candidate_and_one_live_copy_needs_no_pick() {
    use kaanal_core::{ChaosKey, ChaosLayer, PlanItem, sample_plan};
    let (t, e) = build(&cfg(", replicas: 3, balance: least-conn"));
    let db = t.id("db").unwrap();
    let ep = e.get(KEY).unwrap();
    let req = kaanal_core::ChainRequest {
        endpoint: ep,
        n: 0,
        seed: 42,
        pin_delay: std::time::Duration::ZERO,
        req_bytes: 0,
        resp_bytes: 0,
    };
    let down = |copies: &[u16]| {
        let mut s = kaanal_core::ChaosState::default();
        for c in copies {
            s.set(
                ChaosKey::Replica(db, *c),
                ChaosLayer {
                    down: Some(true),
                    ..ChaosLayer::default()
                },
            );
        }
        s.compile()
    };
    let (p, _) = sample_plan(&t, &down(&[1]), &req);
    let cands: Vec<u64> = p
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Pick { cands, .. } => Some(*cands),
            _ => None,
        })
        .collect();
    assert_eq!(cands, [0b101], "copy 1 is down");
    let (p, _) = sample_plan(&t, &down(&[0, 1]), &req);
    assert!(!p.items.iter().any(|i| matches!(i, PlanItem::Pick { .. })));
    let hop_copy = p.items.iter().find_map(|i| match i {
        PlanItem::Hop(h) if h.node == db => Some(h.replica),
        _ => None,
    });
    assert_eq!(hop_copy, Some(2), "the only live copy");
}
