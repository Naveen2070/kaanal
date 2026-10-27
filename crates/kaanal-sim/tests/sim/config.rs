//! Config validation and counters: bad configs, absurd values, too many streams, bounded queues.

use crate::common::*;
use kaanal_core::{ChainResult, ChaosState, NodeChaos};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

#[test]
fn rejects_bad_config() {
    let (t, e) = topo(QUEUED);
    let ok = || one_stream(10.0, 10);
    let cases: Vec<(SimConfig, &str)> = vec![
        (
            {
                let mut c = ok();
                c.streams.clear();
                c
            },
            "no streams",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 0.0;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = f64::NAN;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 1e7;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 1e-7;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].endpoint_key = "GET /nope".into();
                c
            },
            "unknown endpoint",
        ),
        (
            {
                let mut c = ok();
                c.duration = Duration::ZERO;
                c
            },
            "duration",
        ),
        (
            {
                let mut c = ok();
                c.warmup = c.duration;
                c
            },
            "warmup",
        ),
        (
            {
                let mut c = ok();
                c.duration = Duration::from_hours(400 * 24);
                c
            },
            "365 days",
        ),
        (
            {
                let mut c = ok();
                let s = c.streams[0].clone();
                c.streams.push(s);
                c
            },
            "duplicate",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 100_000.0;
                c.duration = Duration::from_secs(100);
                c
            },
            "workload too large",
        ),
    ];
    for (cfg, want) in cases {
        let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
        assert!(err.contains(want), "want '{want}' in: {err}");
    }
}

#[test]
fn request_counters_start_at_zero_and_run_on_consecutively() {
    // Live mode's per-endpoint counter starts at 0; the zero-load test replays each record's own `n`,
    // so this is what pins the sim to live's numbering.
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 20)).unwrap();
    let ns: Vec<u64> = rep.requests.iter().map(|r| r.n).collect();
    assert_eq!(ns, (0..ns.len() as u64).collect::<Vec<_>>());
}

#[test]
fn utilization_divides_by_the_slot_count() {
    // 2 slots, 10 ms exponential service, 100 req/s: 1.0 busy slot on average, so 0.5 per slot.
    let (t, e) = topo(&QUEUED.replace("concurrency: 1", "concurrency: 2"));
    let mut cfg = one_stream(100.0, 200);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let u = rep.nodes[t.id("app").unwrap()].utilization;
    assert!((u - 0.5).abs() < 0.03, "utilization {u}");
}

#[test]
fn hand_edited_absurd_service_time_is_rejected_not_a_panic() {
    let (mut t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    t.nodes[app].proc = kaanal_core::Dist::Fixed(Duration::MAX);
    let err = run(&t, &e, &ChaosState::default(), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("node 'app'") && err.contains("10h"), "{err}");
    t.nodes[app].proc = kaanal_core::Dist::Exp {
        mean: Duration::MAX,
    };
    assert!(run(&t, &e, &ChaosState::default(), &one_stream(1.0, 10)).is_err());
}

#[test]
fn clock_overflow_is_an_error_not_a_silently_wrong_report() {
    // 9 h per request through one slot: 600k arrivals queue up to ~5.4e6 h = 6e2 years > u64 ns.
    let (mut t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    t.nodes[app].proc = kaanal_core::Dist::Fixed(Duration::from_hours(9));
    let err = run(&t, &e, &ChaosState::default(), &one_stream(1000.0, 600)).unwrap_err();
    assert!(err.contains("clock overflow"), "{err}");
}

#[test]
fn absurd_chaos_latency_is_rejected_not_a_panic() {
    // The live limits (R6.9): latency_add over 5 min, or a non-5xx status, is an error.
    let (t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    let with = |c: NodeChaos| ChaosState::from([(app, c)]);
    let cap = NodeChaos {
        latency_add: Duration::from_secs(300),
        ..NodeChaos::default()
    };
    assert!(run(&t, &e, &with(cap.clone()), &one_stream(1.0, 10)).is_ok());
    let over = NodeChaos {
        latency_add: Duration::from_secs(300) + Duration::from_millis(1),
        ..NodeChaos::default()
    };
    let err = run(&t, &e, &with(over), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("latency_add"), "{err}");
    let teapot = NodeChaos {
        error_status: 418,
        ..cap
    };
    let err = run(&t, &e, &with(teapot), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("error_status"), "{err}");
}

#[test]
fn too_many_streams_are_rejected_not_truncated() {
    let (t, e) = topo(QUEUED);
    let mut cfg = one_stream(1.0, 10);
    cfg.streams = (0..65_536)
        .map(|i| Stream {
            endpoint_key: format!("GET /s{i}"),
            rate_per_s: 1.0,
            req_bytes: 0,
            resp_bytes: 0,
        })
        .collect();
    let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
    assert!(err.contains("at most 65535 streams"), "{err}");
}

#[test]
fn bounded_queue_and_rate_limit_counts_match_the_failed_requests() {
    // 120/s offered into 80 rps: the limiter refuses a third (429) and passes smoothed 80/s (load 0.8)
    // to a one-slot node with a queue of 2, which then overflows (503) on its own.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 80, burst: 5 } }\n  app: { type: service, concurrency: { limit: 1, queue: 2 }, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";
    let (t, e) = topo(yaml);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(120.0, 30)).unwrap();
    let (gw, app) = (t.id("gw").unwrap(), t.id("app").unwrap());
    let by = |status: u16| {
        rep.requests
            .iter()
            .filter(|r| matches!(r.result, ChainResult::Err { status: s, .. } if s == status))
            .count() as u64
    };
    assert!(by(429) > 100 && by(503) > 100, "{} {}", by(429), by(503));
    assert_eq!(rep.nodes[gw].limited, by(429));
    assert_eq!(rep.nodes[app].overflowed, by(503));
    assert!(rep.nodes[app].max_queue <= 2, "the queue cap holds");
    assert!(rep.requests.iter().all(|r| match r.result {
        ChainResult::Err {
            status: 429,
            retry_after,
            ..
        } => retry_after.is_some(),
        _ => true,
    }));
}
