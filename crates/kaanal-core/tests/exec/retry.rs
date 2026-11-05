//! Retries in the plan (RULES 82-84, step 7a): a call with `retry` becomes a `Retry` scope of
//! pre-sampled attempts, decided at zero load by the sampler. Off unless written. The oracles are the
//! published models: the geometric series for the calls a retry adds (`sum p^k`), full jitter's bound
//! (Brooker 2015) and RFC 9110's idempotent methods for what retries.

use crate::common::*;
use kaanal_core::{
    ChainOutcome, ChainResult, ChaosState, Config, ErrorKind, FailKind, Hop, NodeChaos, PlanItem,
    Topology,
};
use std::time::Duration;

/// A gateway, a service and a database; `retry` is written on the service's edge to the db (empty =
/// none), `gw` extra fields on the gateway and `chain` the endpoint's chain.
fn cfg(retry: &str, gw: &str, chain: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway{gw} }}\n  app: {{ type: service }}\n  files: {{ type: storage }}\n  db: {{ type: database, ops: {{ read: {{ p50: 4ms, p99: 20ms }}, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 2ms{retry} }}\n  - {{ from: app, to: files, net: 2ms{retry} }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: {chain}\n"
    )
}

const READ: &str = "[{ call: db, op: read }]";

fn retry(body: &str) -> String {
    format!(", retry: {{ {body} }}")
}

fn out(t: &Topology, e: &kaanal_core::Endpoints, chaos: &ChaosState, n: u64) -> ChainOutcome {
    run(t, e, chaos, n)
}

fn failing(t: &Topology, rate: f64, kind: ErrorKind) -> ChaosState {
    chaos_on(
        t,
        "db",
        NodeChaos {
            error_rate: rate,
            error_kind: kind,
            ..NodeChaos::default()
        },
    )
}

/// The hops of plan items `r`.
fn hops_in(got: &ChainOutcome, r: std::ops::Range<usize>) -> Vec<Hop> {
    got.plan.items[r]
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h.clone()),
            _ => None,
        })
        .collect()
}

/// `(marker index, backoff, size)` of each attempt of the first retry scope.
fn attempts(got: &ChainOutcome) -> Vec<(usize, Duration, usize)> {
    got.plan
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, x)| match x {
            PlanItem::Attempt { backoff, size } => Some((i, *backoff, *size)),
            _ => None,
        })
        .collect()
}

fn retry_scopes(got: &ChainOutcome) -> Vec<u8> {
    got.plan
        .items
        .iter()
        .filter_map(|x| match x {
            PlanItem::Retry { attempts, .. } => Some(*attempts),
            _ => None,
        })
        .collect()
}

#[test]
fn nothing_changes_while_retry_is_unset() {
    let (t, e) = build(&cfg("", "", READ));
    for n in 0..200 {
        let got = out(&t, &e, &ChaosState::default(), n);
        assert_eq!(got.plan.scoped.len(), 0);
        assert!(
            got.plan
                .items
                .iter()
                .all(|i| !matches!(i, PlanItem::Retry { .. } | PlanItem::Attempt { .. })),
            "call {n} has a retry scope"
        );
    }
}

#[test]
fn the_first_attempt_draws_what_the_call_draws_without_retry() {
    let (t0, e0) = build(&cfg("", "", READ));
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", READ));
    for n in 0..100 {
        let plain = out(&t0, &e0, &ChaosState::default(), n);
        let with = out(&t, &e, &ChaosState::default(), n);
        // the call succeeds first time: the client sees exactly the hops it saw without retry
        assert_eq!(with.hops, plain.hops, "call {n}");
        assert_eq!(with.result, ChainResult::Ok);
        assert_eq!(with.total(), plain.total());
        // inside the plan, attempt 0 is the plain call's hops
        let a = attempts(&with);
        let first = hops_in(&with, a[0].0 + 1..a[0].0 + 1 + a[0].2);
        let from = plain.hops.iter().position(|h| *h == first[0]).unwrap();
        assert_eq!(first, plain.hops[from..], "call {n}");
    }
}

#[test]
fn attempts_after_the_first_success_are_sampled_for_the_engine_but_never_ran() {
    let (t, e) = build(&cfg(&retry("max_attempts: 4"), "", READ));
    let got = out(&t, &e, &ChaosState::default(), 0);
    assert_eq!(retry_scopes(&got), [4]);
    assert_eq!(attempts(&got).len(), 4, "every attempt is in the plan");
    assert!(got.plan.scoped.is_empty(), "none failed at zero load");
    // ...but only the first shows: the later ones' hops are not in what the client saw
    let all_hops = got
        .plan
        .items
        .iter()
        .filter(|i| matches!(i, PlanItem::Hop(_)))
        .count();
    assert!(got.hops.len() < all_hops);
}

#[test]
fn a_callee_that_always_fails_gets_every_attempt_and_its_last_failure_stands() {
    let (t, e) = build(&cfg(&retry("max_attempts: 3, jitter: none"), "", READ));
    let chaos = failing(&t, 1.0, ErrorKind::Status);
    let got = out(&t, &e, &chaos, 0);
    assert_eq!(retry_scopes(&got), [3]);
    let a = attempts(&got);
    assert_eq!(a.len(), 3);
    // jitter none: the waits are exactly 50 ms and 100 ms (base x 2^k), none before the first attempt
    assert_eq!(
        a.iter().map(|x| x.1).collect::<Vec<_>>(),
        [Duration::ZERO, ms(50), ms(100)]
    );
    assert_eq!(got.plan.scoped.len(), 3, "all three failed");
    let ChainResult::Err {
        node, status, kind, ..
    } = got.result
    else {
        panic!("{:?}", got.result)
    };
    assert_eq!(
        (node, status, kind),
        (t.id("db").unwrap(), 503, FailKind::Status)
    );
    // each retry starts its backoff after the failure before it
    for w in 1..3 {
        let prev = got
            .plan
            .scoped
            .iter()
            .find(|s| s.attempt == a[w - 1].0)
            .unwrap();
        let first = hops_in(&got, a[w].0 + 1..a[w].0 + 1 + a[w].2)[0].start;
        assert_eq!(first, prev.at + a[w].1, "attempt {w}");
    }
    // the request ends at the last failure, so the backoffs are inside its latency
    assert!(got.total() >= ms(150), "{:?}", got.total());
}

#[test]
fn full_jitter_stays_inside_the_backoff() {
    let (t, e) = build(&cfg(&retry("max_attempts: 5"), "", READ));
    let chaos = failing(&t, 1.0, ErrorKind::Status);
    for n in 0..200 {
        let got = out(&t, &e, &chaos, n);
        let a = attempts(&got);
        for (k, (_, wait, _)) in a.iter().enumerate().skip(1) {
            // base 50 ms doubling: 50, 100, 200, 400
            let bound = ms(50 << (k - 1));
            assert!(*wait <= bound, "attempt {k}: {wait:?} > {bound:?}");
        }
    }
}

#[test]
fn a_failure_that_is_not_transient_is_not_retried() {
    // a garbled response (Malformed) is a verdict, not a blip
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", READ));
    let chaos = failing(&t, 1.0, ErrorKind::Malformed);
    let got = out(&t, &e, &chaos, 0);
    assert_eq!(retry_scopes(&got), [1], "sampling stops after it");
    assert_eq!(got.plan.scoped.len(), 1);
    let ChainResult::Err { kind, .. } = got.result else {
        panic!()
    };
    assert_eq!(kind, FailKind::Malformed);
}

#[test]
fn a_later_attempt_can_succeed() {
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", READ));
    let chaos = failing(&t, 0.5, ErrorKind::Status);
    let mut seen = 0;
    for n in 0..200 {
        let got = out(&t, &e, &chaos, n);
        if got.plan.scoped.len() == 1 && got.result == ChainResult::Ok {
            seen += 1;
            // the failed attempt is cut at its failure; the second attempt's hops are in what the client saw
            let first_fail = got.plan.scoped[0].at;
            assert!(got.hops.iter().any(|h| h.start >= first_fail));
        }
    }
    assert!(
        seen > 20,
        "saw {seen} requests rescued by their second attempt"
    );
}

#[test]
fn calls_per_request_follow_the_geometric_series() {
    // independent failure probability p per attempt, at most n tries: E[attempts] = sum_{k<n} p^k
    // (the offered load a retry adds, the amplification of Nygard and the SRE book).
    for (p, n, tries) in [(0.5, 3u32, "3"), (0.2, 5, "5"), (0.8, 4, "4")] {
        let (t, e) = build(&cfg(&retry(&format!("max_attempts: {tries}")), "", READ));
        let chaos = failing(&t, p, ErrorKind::Status);
        let runs = 20_000u64;
        let mut reached = 0u64;
        for i in 0..runs {
            let got = out(&t, &e, &chaos, i);
            reached += got.plan.scoped.len() as u64 + u64::from(got.result == ChainResult::Ok);
        }
        let want: f64 = (0..n)
            .map(|k| f64::powi(p, i32::try_from(k).unwrap()))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let got = reached as f64 / runs as f64;
        assert!(
            (got - want).abs() / want < 0.02,
            "p {p}, {n} tries: {got} vs {want}"
        );
    }
}

#[test]
fn a_wait_longer_than_the_callers_deadline_loses_to_the_deadline() {
    // the gateway gives each request 10 ms; the retry would wait 1 s: the deadline wins
    let (t, e) = build(&cfg(
        &retry("max_attempts: 3, backoff: 1s, jitter: none"),
        ", timeout: 10ms",
        READ,
    ));
    let chaos = failing(&t, 1.0, ErrorKind::Status);
    let got = out(&t, &e, &chaos, 0);
    let ChainResult::Err {
        node, kind, status, ..
    } = got.result
    else {
        panic!("{:?}", got.result)
    };
    assert_eq!(
        (node, kind, status),
        (t.id("nginx").unwrap(), FailKind::Timeout, 504)
    );
    assert_eq!(
        got.total(),
        ms(10),
        "the client is answered at the deadline"
    );
}

#[test]
fn a_write_retries_only_when_idempotent_or_asked() {
    let chain = "[{ call: db, op: write }]";
    // a write to a database is not idempotent: no scope without `writes`
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", chain));
    assert_eq!(
        retry_scopes(&out(&t, &e, &ChaosState::default(), 0)).len(),
        0
    );
    let (t, e) = build(&cfg(&retry("max_attempts: 3, writes: true"), "", chain));
    assert_eq!(retry_scopes(&out(&t, &e, &ChaosState::default(), 0)), [3]);
    // a write to a storage node is a PUT: idempotent (RFC 9110 9.2.2), it retries by default (RULES 84)
    let (t, e) = build(&cfg(
        &retry("max_attempts: 3"),
        "",
        "[{ call: files, op: write }]",
    ));
    assert_eq!(retry_scopes(&out(&t, &e, &ChaosState::default(), 0)), [3]);
    // and a read retries without being asked
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", READ));
    assert_eq!(retry_scopes(&out(&t, &e, &ChaosState::default(), 0)), [3]);
}

#[test]
fn a_step_retry_beats_its_edges() {
    let (t, e) = build(&cfg(
        &retry("max_attempts: 5"),
        "",
        "[{ call: db, op: read, retry: { max_attempts: 2 } }]",
    ));
    assert_eq!(retry_scopes(&out(&t, &e, &ChaosState::default(), 0)), [2]);
    // a step can add one where the edge has none
    let (t, e) = build(&cfg(
        "",
        "",
        "[{ call: db, op: read, retry: { max_attempts: 4 } }]",
    ));
    assert_eq!(retry_scopes(&out(&t, &e, &ChaosState::default(), 0)), [4]);
}

#[test]
fn the_same_inputs_make_the_same_plan() {
    let (t, e) = build(&cfg(&retry("max_attempts: 4"), "", READ));
    let chaos = failing(&t, 0.6, ErrorKind::Status);
    for n in 0..50 {
        assert_eq!(out(&t, &e, &chaos, n), out(&t, &e, &chaos, n));
    }
}

#[test]
fn a_retry_field_out_of_range_is_a_load_error() {
    for (body, field) in [
        ("max_attempts: 1", "retry.max_attempts"),
        ("max_attempts: 6", "retry.max_attempts"),
        ("backoff: 0s", "retry.backoff"),
        ("budget: { max_tokens: -1 }", "retry.budget.max_tokens"),
    ] {
        let err = Config::from_yaml(&cfg(&retry(body), "", READ)).expect_err(body);
        assert!(err.to_string().contains(field), "{body}: {err}");
    }
}

#[test]
fn the_budget_travels_in_the_plan_with_its_edge_and_caller_replica() {
    let (t, e) = build(&cfg(
        &retry("max_attempts: 3, budget: { max_tokens: 20, token_ratio: 0.25 }"),
        "",
        READ,
    ));
    let got = out(&t, &e, &ChaosState::default(), 0);
    let Some(PlanItem::Retry {
        budget: Some(b), ..
    }) = got
        .plan
        .items
        .iter()
        .find(|i| matches!(i, PlanItem::Retry { .. }))
    else {
        panic!("{:?}", got.plan.items)
    };
    // edge 1 is app -> db (config order), the caller's only replica is 0; A6 in thousandths of a token
    assert_eq!(
        (b.edge, b.replica, b.max_milli, b.ratio_milli),
        (1, 0, 20_000, 250)
    );
    // no budget written, none in the plan
    let (t, e) = build(&cfg(&retry("max_attempts: 3"), "", READ));
    let plain = out(&t, &e, &ChaosState::default(), 0);
    assert!(plain.plan.items.iter().all(|i| !matches!(
        i,
        PlanItem::Retry {
            budget: Some(_),
            ..
        }
    )));
}

#[test]
fn attempts_that_never_ran_do_not_move_the_replica_picks_of_the_rest_of_the_request() {
    // two reads of a two-copy database in round robin: the second read takes the other copy. The
    // first read's unused extra attempts are in the plan, but they must not advance the counter.
    let chain = "[{ call: db, op: read }, { call: db, op: read }]";
    let replicas = |s: String| {
        s.replace(
            "db: { type: database,",
            "db: { type: database, replicas: 2,",
        )
    };
    let (t0, e0) = build(&replicas(cfg("", "", chain)));
    let (t, e) = build(&replicas(cfg(&retry("max_attempts: 4"), "", chain)));
    let copies = |got: &ChainOutcome| -> Vec<u16> {
        got.hops
            .iter()
            .filter(|h| matches!(h.kind, kaanal_core::HopKind::Op(_)))
            .map(|h| h.replica)
            .collect()
    };
    for n in 0..50 {
        let plain = out(&t0, &e0, &ChaosState::default(), n);
        let with = out(&t, &e, &ChaosState::default(), n);
        assert_eq!(copies(&with), copies(&plain), "call {n}");
        assert_eq!(with.hops, plain.hops, "call {n}");
    }
}
