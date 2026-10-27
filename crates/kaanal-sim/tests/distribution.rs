//! Zero-load drift check, statistical form (spec section 5): the simulator's per-hop latency
//! distributions match live `execute()` samples drawn for a disjoint range of `n`.
//! Today the simulator copies `execute()`'s hops, so this pins the RNG stream and the
//! per-hop numbering, not queueing. It keeps its meaning once the simulator's RNG stream diverges from live's (M3).

mod common;

use common::{CHAIN_ROUTES, CHAINS, compiled};
use kaanal_core::{ChainRequest, ChaosState, Config, Hop, execute};
use kaanal_sim::{SimConfig, Stream, run};
use std::collections::BTreeMap;
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

/// Durations in ms grouped by `(node, kind)`: shapes vary with cache hits and repeats, so
/// hop indices do not line up between requests.
fn by_kind<'a>(hops: impl Iterator<Item = &'a Hop>) -> BTreeMap<(usize, String), Vec<f64>> {
    let mut out: BTreeMap<(usize, String), Vec<f64>> = BTreeMap::new();
    for h in hops {
        out.entry((h.node, format!("{:?}", h.kind)))
            .or_default()
            .push(h.dur.as_secs_f64() * 1e3);
    }
    out
}

#[test]
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts are a few thousand"
)]
fn zero_load_matches_live_distribution() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_routes = vec![("GET /users/{id}", "app")];
    let configs = [
        (
            "fixture",
            std::fs::read_to_string(root.join("../kaanal-server/tests/fixtures/kaanal.yaml"))
                .unwrap(),
            app_routes.clone(),
        ),
        (
            "example",
            std::fs::read_to_string(root.join("../../examples/nginx-server/kaanal.yaml")).unwrap(),
            app_routes,
        ),
        ("chains", CHAINS.to_string(), CHAIN_ROUTES.to_vec()),
    ];
    for (p, yaml, routes) in configs {
        let seed = Config::from_yaml(&yaml).unwrap().seed;
        let (t, e) = compiled(&yaml, &routes);
        let chaos = ChaosState::default();
        let key = "GET /users/{id}";
        let sc = SimConfig {
            streams: vec![Stream {
                endpoint_key: key.into(),
                rate_per_s: 1.0,
                req_bytes: 0,
                resp_bytes: 0,
            }],
            duration: Duration::from_secs(5_000),
            warmup: Duration::ZERO,
            seed,
        };
        let rep = run(&t, &e, &chaos, &sc).unwrap();
        let live: Vec<_> = (1_000_000..1_005_000u64)
            .map(|n| {
                execute(
                    &t,
                    &chaos.compile(),
                    &ChainRequest {
                        endpoint: e.get(key).unwrap(),
                        n,
                        seed,
                        pin_delay: std::time::Duration::ZERO,
                        req_bytes: 0,
                        resp_bytes: 0,
                    },
                )
            })
            .collect();
        let sim_ms = by_kind(
            rep.requests
                .iter()
                .flat_map(|r| r.hops.iter().map(|(h, _)| h)),
        );
        let live_ms = by_kind(live.iter().flat_map(|o| o.hops.iter()));
        let mut compared = 0;
        // A kind that is common on one side must exist on the other.
        for (side, a, b) in [("sim", &sim_ms, &live_ms), ("live", &live_ms, &sim_ms)] {
            for (k, xs) in a {
                assert!(
                    xs.len() < 500 || b.contains_key(k),
                    "{p}: {k:?} is common in {side} but absent from the other"
                );
            }
        }
        for (k, xs) in &sim_ms {
            let Some(ys) = live_ms.get(k) else { continue };
            if xs.len() < 500 || ys.len() < 500 {
                continue;
            }
            let (n_sim, n_live) = (xs.len() as f64, ys.len() as f64);
            // α = 0.001 critical value for the two-sample KS test.
            let crit = 1.949 * ((n_sim + n_live) / (n_sim * n_live)).sqrt();
            let stat = ks_d(xs.clone(), ys.clone());
            assert!(stat < crit, "{p}: {k:?}: D = {stat:.4} ≥ {crit:.4}");
            compared += 1;
        }
        assert!(compared > 0, "{p}: nothing was compared");
    }
}
