//! A circuit breaker in the plan (step 7c): a call over an edge with `breaker` becomes a scope the engine
//! asks before each attempt. With no `retry` it is a scope of one attempt that draws what the call draws,
//! so no number moves; the breaker itself only acts under load, in the engine.

use crate::common::*;
use kaanal_core::{
    BreakerRef, ChainOutcome, ChainResult, ChaosState, ErrorKind, NodeChaos, PlanItem, Topology,
};

/// A gateway, a service and a database; `edge` is extra fields on the service's edge to the db.
fn cfg(edge: &str, chain: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  db: {{ type: database, ops: {{ read: {{ p50: 4ms, p99: 20ms }}, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 2ms{edge} }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: {chain}\n"
    )
}

const READ: &str = "[{ call: db, op: read }]";
const WRITE: &str = "[{ call: db, op: write }]";
const BREAKER: &str = ", breaker: {}";

fn out(t: &Topology, e: &kaanal_core::Endpoints, chaos: &ChaosState, n: u64) -> ChainOutcome {
    run(t, e, chaos, n)
}

/// `(attempts, breaker)` of each retry scope of the plan.
fn scopes(got: &ChainOutcome) -> Vec<(u8, Option<BreakerRef>)> {
    got.plan
        .items
        .iter()
        .filter_map(|x| match x {
            PlanItem::Retry {
                attempts, breaker, ..
            } => Some((*attempts, *breaker)),
            _ => None,
        })
        .collect()
}

fn failing(t: &Topology) -> ChaosState {
    chaos_on(
        t,
        "db",
        NodeChaos {
            error_rate: 0.5,
            error_kind: ErrorKind::Status,
            ..NodeChaos::default()
        },
    )
}

#[test]
fn an_edge_without_a_breaker_makes_no_scope() {
    let (t, e) = build(&cfg("", READ));
    for n in 0..100 {
        assert_eq!(scopes(&out(&t, &e, &ChaosState::default(), n)), []);
    }
}

#[test]
fn a_breaker_alone_is_a_scope_of_one_attempt_that_changes_no_number() {
    let (t0, e0) = build(&cfg("", READ));
    let (t, e) = build(&cfg(BREAKER, READ));
    let (c0, c) = (failing(&t0), failing(&t));
    for n in 0..300 {
        let (plain, with) = (out(&t0, &e0, &c0, n), out(&t, &e, &c, n));
        // the edge is app -> db, the second in config order; the caller's only replica is 0
        assert_eq!(
            scopes(&with),
            [(
                1,
                Some(BreakerRef {
                    edge: 1,
                    replica: 0
                })
            )],
            "call {n}"
        );
        assert_eq!(with.hops, plain.hops, "call {n}");
        assert_eq!(with.result, plain.result, "call {n}");
        assert_eq!(with.total(), plain.total(), "call {n}");
    }
}

#[test]
fn a_breaker_beside_a_retry_guards_every_attempt() {
    let (t, e) = build(&cfg(", retry: { max_attempts: 3 }, breaker: {}", READ));
    let got = out(&t, &e, &ChaosState::default(), 0);
    assert_eq!(
        scopes(&got),
        [(
            3,
            Some(BreakerRef {
                edge: 1,
                replica: 0
            })
        )]
    );
}

#[test]
fn a_write_that_retry_skips_still_goes_through_the_breaker() {
    // a write to a database is not retried without `retry.writes`, but its failures still count
    let (t, e) = build(&cfg(", retry: { max_attempts: 3 }, breaker: {}", WRITE));
    let got = out(&t, &e, &ChaosState::default(), 0);
    assert_eq!(
        scopes(&got),
        [(
            1,
            Some(BreakerRef {
                edge: 1,
                replica: 0
            })
        )]
    );
    assert_eq!(got.result, ChainResult::Ok);
}
