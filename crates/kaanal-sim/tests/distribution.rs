//! Zero-load drift check, statistical form (spec §5): the simulator's per-hop latency
//! distributions match live `execute()` samples drawn for a disjoint range of `n`.
//! Today the simulator copies `execute()`'s hops, so this pins the RNG stream and the
//! per-hop numbering, not queueing. It keeps its meaning once the simulator's RNG stream diverges from live's (M3).

use kaanal_core::{ChainRequest, ChaosState, Config, Op, Topology, execute};
use kaanal_sim::{SimConfig, Stream, run};
use std::path::Path;
use std::time::Duration;

/// Two-sample Kolmogorov–Smirnov statistic D (handles ties).
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts are a few thousand"
)]
fn ks_d(mut xs: Vec<f64>, mut ys: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let (n_xs, n_ys) = (xs.len() as f64, ys.len() as f64);
    let (mut ix, mut iy, mut gap) = (0, 0, 0.0f64);
    while ix < xs.len() && iy < ys.len() {
        let v = xs[ix].min(ys[iy]);
        while ix < xs.len() && xs[ix] <= v {
            ix += 1;
        }
        while iy < ys.len() && ys[iy] <= v {
            iy += 1;
        }
        gap = gap.max((ix as f64 / n_xs - iy as f64 / n_ys).abs());
    }
    gap
}

#[test]
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts are a few thousand"
)]
fn zero_load_matches_live_distribution() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for p in [
        "../kaanal-server/tests/fixtures/kaanal.yaml",
        "../../examples/nginx-server/kaanal.yaml",
    ] {
        let cfg = Config::from_yaml(&std::fs::read_to_string(root.join(p)).unwrap()).unwrap();
        let t = Topology::build(&cfg).unwrap();
        let chaos = ChaosState::default();
        let key = "GET /users/{id}";
        let sc = SimConfig {
            streams: vec![Stream {
                entry: t.id("app").unwrap(),
                op: Op::Read,
                endpoint_key: key.into(),
                rate_per_s: 1.0,
            }],
            duration: Duration::from_secs(5_000),
            warmup: Duration::ZERO,
            seed: cfg.seed,
        };
        let rep = run(&t, &chaos, &sc).unwrap();
        let live: Vec<_> = (1_000_000..1_005_000u64)
            .map(|n| {
                execute(
                    &t,
                    &chaos,
                    &ChainRequest {
                        entry: t.id("app"),
                        op: Op::Read,
                        endpoint_key: key,
                        n,
                        seed: cfg.seed,
                    },
                )
            })
            .collect();
        // Without chaos every chain has the same shape, so hop i is the same (node, kind).
        let hops = live[0].hops.len();
        for i in 0..hops {
            let sim_ms: Vec<f64> = rep
                .requests
                .iter()
                .map(|r| r.hops[i].0.dur.as_secs_f64() * 1e3)
                .collect();
            let live_ms: Vec<f64> = live
                .iter()
                .map(|o| o.hops[i].dur.as_secs_f64() * 1e3)
                .collect();
            let (n_sim, n_live) = (sim_ms.len() as f64, live_ms.len() as f64);
            // α = 0.001 critical value for the two-sample KS test.
            let crit = 1.949 * ((n_sim + n_live) / (n_sim * n_live)).sqrt();
            let hop = &live[0].hops[i];
            let stat = ks_d(sim_ms, live_ms);
            assert!(
                stat < crit,
                "{p}: hop {i} ({:?} at node {}): D = {stat:.4} ≥ {crit:.4}",
                hop.kind,
                hop.node
            );
        }
    }
}
