//! Fair sharing of pipes in the simulator (RULES 62, step 6a): a big transfer no longer blocks a
//! small one, one node's card is shared by every wire out of it, and a topology without a rate is
//! untouched.

use crate::common::*;
use crate::shared::compiled;
use kaanal_core::{ChaosState, Config, HopKind};
use kaanal_sim::{SimConfig, Stream};
use std::time::Duration;

/// A gateway that calls `app` over a 1 MB/s wire.
const WIRE: &str = "version: 1\nheader_bytes: 0\nseed: 5\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app, bandwidth: 1000000 }\n";

fn stream(key: &str, rate: f64, req: u64) -> Stream {
    Stream {
        endpoint_key: key.into(),
        rate_per_s: rate,
        req_bytes: req,
        resp_bytes: 0,
    }
}

fn run_streams(src: &str, streams: Vec<Stream>, secs: u64) -> kaanal_sim::SimReport {
    let seed = Config::from_yaml(src).unwrap().seed;
    let (t, e) = compiled(src, &APP_ROUTES);
    let cfg = SimConfig {
        streams,
        duration: Duration::from_secs(secs),
        warmup: Duration::from_secs(secs / 10),
        seed,
    };
    kaanal_sim::run(&t, &e, &ChaosState::default(), &cfg).unwrap()
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / f64::from(u32::try_from(xs.len()).unwrap())
}

fn p99(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() * 99 / 100]
}

fn latencies(rep: &kaanal_sim::SimReport, stream: usize) -> Vec<f64> {
    rep.requests
        .iter()
        .filter(|r| r.stream == stream)
        .map(|r| r.latency.as_secs_f64())
        .collect()
}

#[test]
fn a_small_call_is_not_held_up_by_a_big_upload_on_the_same_wire() {
    // 500 kB uploads once a second (half a second of wire each) and 1 kB calls 40 times a second.
    // FIFO made a small call wait behind a whole upload (p99 about 0.4 s); sharing makes it a few
    // milliseconds (1 ms alone, 2 ms while an upload shares the wire).
    let rep = run_streams(
        WIRE,
        vec![
            stream("POST /orders", 1.0, 500_000),
            stream("GET /users/{id}", 40.0, 1_000),
        ],
        200,
    );
    let small = p99(&mut latencies(&rep, 1));
    let big = latencies(&rep, 0);
    let big_mean = mean(&big);
    assert!(small < 0.02, "small p99 {small} s");
    assert!(
        big_mean > 0.5,
        "the upload still takes its half second: {big_mean} s"
    );
}

#[test]
fn the_time_lost_to_sharing_shows_as_the_wait_of_a_transfer_hop() {
    let rep = run_streams(
        WIRE,
        vec![
            stream("POST /orders", 1.0, 500_000),
            stream("GET /users/{id}", 40.0, 1_000),
        ],
        200,
    );
    let waited = rep
        .requests
        .iter()
        .flat_map(|r| r.hops.iter())
        .filter(|(h, w)| matches!(h.kind, HopKind::Link { .. }) && !w.is_zero())
        .count();
    assert!(waited > 0, "transfers that shared the wire lost time");
    let alone = rep
        .requests
        .iter()
        .filter(|r| r.stream == 1)
        .flat_map(|r| r.hops.iter())
        .filter(|(h, _)| matches!(h.kind, HopKind::Link { .. }))
        .map(|(h, _)| h.dur)
        .max()
        .unwrap();
    assert_eq!(
        alone,
        Duration::from_millis(1),
        "1 kB at 1 MB/s at zero load"
    );
}

#[test]
fn a_run_without_any_rate_has_no_transfer_hops_and_the_same_numbers_twice() {
    let src = "version: 1\nseed: 5\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: { mean: 2ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n";
    let streams = || vec![stream("GET /users/{id}", 30.0, 10_000)];
    let a = run_streams(src, streams(), 60);
    let b = run_streams(src, streams(), 60);
    assert!(
        a.requests
            .iter()
            .flat_map(|r| r.hops.iter())
            .all(|(h, _)| !matches!(h.kind, HopKind::Link { .. } | HopKind::Nic { .. })),
        "no rate means no transfer hop"
    );
    let lat = |r: &kaanal_sim::SimReport| r.requests.iter().map(|x| x.latency).collect::<Vec<_>>();
    assert_eq!(lat(&a), lat(&b), "deterministic");
}

#[test]
fn one_card_is_shared_by_every_wire_out_of_it() {
    // gw has a 1 MB/s send pipe and two routes through it, 0.5 s of pipe per upload and one
    // upload a second in all (rho 0.5). Sharing the card, the mean call takes E[S] / (1 - rho) =
    // 1.0 s (M/G/1 processor sharing); FIFO would have given 0.75 s.
    let src = "version: 1
header_bytes: 0
seed: 5
nodes:
  gw: { type: gateway, bandwidth: { out: 1000000 } }
  app: { type: service }
edges:
  - { from: gw, to: app }
";
    let rep = run_streams(
        src,
        vec![
            stream("POST /orders", 0.5, 500_000),
            stream("GET /users", 0.5, 500_000),
        ],
        8000,
    );
    let all: Vec<f64> = rep
        .requests
        .iter()
        .map(|r| r.latency.as_secs_f64())
        .collect();
    let mean = mean(&all);
    assert!(
        (mean - 1.0).abs() < 0.08,
        "mean {mean} s, processor-sharing theory 1.0 s"
    );
}
