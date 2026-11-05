//! Characterization of `run` taken BEFORE the replay engine moved to core (B1 Task 0).
//! Each case fingerprints the whole report; the hashes were recorded from the old
//! simulator, so they prove the move changed nothing observable for limit-free
//! configs and `concurrency: N`. Never re-record a hash to make a test pass (R7.5): a
//! differing hash is a behaviour change to explain.
//!
//! Re-recorded once on 2026-10-25 (developer decision, perf-sim): the per-hop RNG changed from
//! `ChaCha8Rng` to `HopRng` (`SplitMix64`) to cut seeding cost, which re-pins every sampled value.
//! "ties" has no random draw and kept its hash. Structure, ordering and the oracle tests are
//! unchanged.

mod common;

use common::{CHAIN_ROUTES, CHAINS, compiled};
use kaanal_core::{ChainResult, ChaosState, NodeChaos, stable_hash};
use kaanal_sim::{SimConfig, SimReport, Stream, run};
use std::fmt::Write as _;
use std::time::Duration;

/// A stable text of everything a report says that B1 must not change.
fn fingerprint(rep: &SimReport) -> u64 {
    let mut s = String::new();
    for r in &rep.requests {
        let _ = write!(
            s,
            "r{}:{}:{}:{}",
            r.stream,
            r.n,
            r.arrival.as_nanos(),
            r.latency.as_nanos()
        );
        match &r.result {
            ChainResult::Ok => s.push_str(":ok"),
            ChainResult::Err { node, status, .. } => {
                let _ = write!(s, ":err{node}/{status}");
            }
        }
        for (h, w) in &r.hops {
            let _ = write!(
                s,
                "|{}:{:?}:{}:{}:{}",
                h.node,
                h.kind,
                h.start.as_nanos(),
                h.dur.as_nanos(),
                w.as_nanos()
            );
        }
        s.push('\n');
    }
    for n in &rep.nodes {
        let _ = writeln!(
            s,
            "n{}:{}:{}:{}",
            n.completed,
            n.mean_wait.as_nanos(),
            n.utilization.to_bits(),
            n.max_queue
        );
    }
    stable_hash(&[s.as_bytes()])
}

fn streams(list: &[(&str, f64)], secs: u64, warmup: u64, seed: u64) -> SimConfig {
    SimConfig {
        streams: list
            .iter()
            .map(|(k, r)| Stream {
                endpoint_key: (*k).into(),
                rate_per_s: *r,
                req_bytes: 0,
                resp_bytes: 0,
            })
            .collect(),
        duration: Duration::from_secs(secs),
        warmup: Duration::from_secs(warmup),
        seed,
    }
}

const QUEUED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";
const TANDEM: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 2, proc: { mean: 10ms } }\n  db: { type: database, concurrency: 1, ops: { read: { mean: 5ms }, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }\n";
const FORKED: &str = "version: 1\nnodes:\n  app: { type: service }\n  a: { type: external-api, proc: 3ms }\n  s: { type: external-api, proc: 4ms, concurrency: 1 }\nedges:\n  - { from: app, to: a }\n  - { from: app, to: s }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ s, s, a ] } ] }\n";
const TIES: &str = "version: 1\nnodes:\n  app: { type: service }\n  p: { type: external-api, proc: 1ms }\n  q: { type: external-api, proc: 2ms }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: p }\n  - { from: app, to: q }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ [ p, p, s ], [ q, a ] ] } ] }\n";
const LOGIN: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  login: { type: auth, concurrency: 1, proc: { mean: 10ms } }\n  users: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: login }\n  - { from: login, to: users }\n";

/// (name, hash recorded from the pre-move simulator). `tandem` and `chains` were re-recorded for
/// step 6b (RULES 68): their edges have a `net`, so about 1 call in 1000 now opens a connection and
/// pays its handshake. Checked: with the default keep-alive at 1,000,000 both hashes are the old ones.
const GOLDEN: [(&str, u64); 6] = [
    ("queued", 10_122_474_828_312_530_245),
    ("tandem", 187_513_312_029_472_560),
    ("forked", 11_009_132_828_405_713_776),
    ("ties", 12_536_427_626_783_952_910),
    ("chains", 1_402_007_245_853_917_768),
    ("login", 4_973_649_437_012_228_117),
];

fn case(name: &str) -> SimReport {
    match name {
        "queued" => {
            let (t, e) = compiled(QUEUED, &[("GET /users/{id}", "app")]);
            let cfg = streams(&[("GET /users/{id}", 80.0)], 20, 2, 42);
            run(&t, &e, &ChaosState::default(), &cfg).unwrap()
        }
        "tandem" => {
            let (t, e) = compiled(TANDEM, &[("GET /users/{id}", "app")]);
            let cfg = streams(&[("GET /users/{id}", 150.0)], 10, 1, 7);
            run(&t, &e, &ChaosState::default(), &cfg).unwrap()
        }
        "forked" => {
            // `a` fails 30% of the time, so queued siblings are cancelled and slots are kept.
            let (t, e) = compiled(FORKED, &[("GET /x", "app")]);
            let flaky = ChaosState::from([(
                t.id("a").unwrap(),
                NodeChaos {
                    error_rate: 0.3,
                    ..NodeChaos::default()
                },
            )]);
            let cfg = streams(&[("GET /x", 40.0)], 10, 1, 5);
            run(&t, &e, &flaky, &cfg).unwrap()
        }
        "ties" => {
            let (t, e) = compiled(TIES, &[("GET /x", "app")]);
            let down = ChaosState::from([(
                t.id("s").unwrap(),
                NodeChaos {
                    down: true,
                    ..NodeChaos::default()
                },
            )]);
            let cfg = streams(&[("GET /x", 0.01)], 10_000, 0, 3);
            run(&t, &e, &down, &cfg).unwrap()
        }
        "chains" => {
            let yaml = CHAINS.replace(
                "db: { type: database, flavor: postgres }",
                "db: { type: database, flavor: postgres, concurrency: 2 }",
            );
            let (t, e) = compiled(&yaml, &CHAIN_ROUTES);
            let flaky = ChaosState::from([(
                t.id("pay").unwrap(),
                NodeChaos {
                    error_rate: 0.1,
                    ..NodeChaos::default()
                },
            )]);
            let cfg = streams(
                &[("GET /users/{id}", 30.0), ("POST /orders", 10.0)],
                10,
                1,
                11,
            );
            run(&t, &e, &flaky, &cfg).unwrap()
        }
        "login" => {
            let (t, e) = compiled(LOGIN, &[]);
            let cfg = streams(&[("POST /login", 50.0)], 10, 0, 3);
            run(&t, &e, &ChaosState::default(), &cfg).unwrap()
        }
        _ => unreachable!("{name}"),
    }
}

#[test]
fn reports_match_the_pre_move_simulator() {
    let mut wrong = Vec::new();
    for (name, want) in GOLDEN {
        let got = fingerprint(&case(name));
        if got != want {
            wrong.push(format!("(\"{name}\", {got}),"));
        }
    }
    assert!(
        wrong.is_empty(),
        "fingerprints differ from the recorded ones; current values:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_case_runs_requests_and_the_fingerprint_is_stable() {
    for (name, _) in GOLDEN {
        let (a, b) = (case(name), case(name));
        assert!(!a.requests.is_empty(), "{name}");
        assert_eq!(fingerprint(&a), fingerprint(&b), "{name} is deterministic");
    }
}
