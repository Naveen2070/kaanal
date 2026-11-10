//! Garbage-collection cost on a node's own work (step 8G, RULES 112): a request's own work takes
//! `1 + g u / (1 - u)` times its drawn time, `u` the live share of the memory when it starts
//! (Appel, 1987: collecting costs about `L / (H - L)` per byte allocated for live data `L` in a
//! heap `H`). The oracle is that formula, not a number the engine printed.

use crate::common::*;
use kaanal_core::HopKind;
use kaanal_core::replay::Engine;

/// 1 GiB of memory, 100 MiB a request, 200 MiB idle, a GC weight of 0.5, ten slots.
const GC: &str = "version: 1\nnodes:\n  app: { type: service, memory: { limit: 1GB, per_request: 100MB, base: 200MB, gc: 0.5 } }\n";
const NO_GC: &str = "version: 1\nnodes:\n  app: { type: service, memory: { limit: 1GB, per_request: 100MB, base: 200MB } }\n";

/// The formula of the model for `live` requests being served.
fn appel(live: f64, g: f64, base_mb: f64, per_mb: f64, limit_mb: f64) -> f64 {
    let u = (base_mb + live * per_mb) / limit_mb;
    1.0 + g * u / (1.0 - u)
}

fn run_n(yaml: &str, n: u64, ms_: u64) -> Vec<f64> {
    let t = topo(yaml);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    for tag in 0..n {
        e.submit(0, plan(vec![hop(app, HopKind::Proc, ms_)]), tag)
            .unwrap();
    }
    let f = run_all(&mut e);
    let mut done: Vec<f64> = (0..n)
        .map(|o| f64::from(u32::try_from(fin(&f, o).done / 1000).unwrap()) / 1000.0)
        .collect();
    done.sort_by(f64::total_cmp);
    done
}

#[test]
fn a_lone_request_takes_the_appel_factor_of_its_time() {
    let got = run_n(GC, 1, 10)[0];
    let want = 10.0 * appel(1.0, 0.5, 200.0, 100.0, 1024.0);
    assert!(
        (got - want).abs() / want < 0.005,
        "{got} ms against {want} ms"
    );
}

#[test]
fn the_more_requests_are_served_the_slower_each_start_is() {
    // three at once, each starts when 1, 2, 3 are in: factors for 1, 2 and 3 live requests
    let done = run_n(GC, 3, 10);
    for (i, got) in done.iter().enumerate() {
        let want = 10.0
            * appel(
                f64::from(u32::try_from(i).unwrap()) + 1.0,
                0.5,
                200.0,
                100.0,
                1024.0,
            );
        assert!(
            (got - want).abs() / want < 0.005,
            "request {i}: {got} against {want}"
        );
    }
}

#[test]
fn no_gc_weight_changes_nothing() {
    for got in run_n(NO_GC, 3, 10) {
        assert!((got - 10.0).abs() < 1e-9, "{got}");
    }
}

#[test]
fn near_the_limit_the_slowdown_is_capped_not_infinite() {
    // memory for 8 requests (1000 MiB less 200 idle, 100 each), nine arrive: the factor of the 8th
    // is large but finite, and the node is not asked for more than 0.99 of its memory
    let done = run_n(
        "version: 1\nnodes:\n  app: { type: service, memory: { limit: 1000MB, per_request: 100MB, base: 200MB, gc: 1 } }\n",
        8,
        10,
    );
    let last = done[7];
    // u = 1000/1000 -> capped at 0.99: 1 + 0.99 / 0.01 = 100
    assert!((last - 1000.0).abs() < 1.0, "{last} ms");
}

#[test]
fn a_gc_weight_must_be_above_zero_and_at_most_ten() {
    for bad in ["0", "-1", "11", "12.5"] {
        let cfg = format!(
            "version: 1\nnodes:\n  app: {{ type: service, memory: {{ limit: 1GB, per_request: 1MB, gc: {bad} }} }}\n"
        );
        assert!(
            kaanal_core::Config::from_yaml(&cfg)
                .map_err(|e| e.to_string())
                .is_err_and(|e| e.contains("gc")),
            "{bad}"
        );
    }
}
