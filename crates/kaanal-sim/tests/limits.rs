//! `Limits`: the caps a caller can lower (the server runs under tighter ones).

#[path = "common/mod.rs"]
mod shared;

use kaanal_core::EffectiveChaos;
use kaanal_sim::{Limits, SimConfig, Stream, run_effective, run_with_limits};
use shared::{CHAIN_ROUTES, CHAINS, compiled};
use std::time::Duration;

fn heavy() -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /users/{id}".into(),
            rate_per_s: 2000.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(300),
        warmup: Duration::from_secs(10),
        seed: 1,
    }
}

#[test]
fn server_limits_reject_a_run_the_default_accepts() {
    let (t, e) = compiled(CHAINS, &CHAIN_ROUTES);
    let cfg = heavy();
    // the default cap lets validation through (the run itself is not executed here: a tiny
    // duration would hide the check), so compare the two verdicts on the same input
    let server = run_with_limits(&t, &e, &EffectiveChaos::default(), &cfg, &Limits::server())
        .expect_err("over the server caps");
    assert!(server.contains("too large"), "{server}");
    let mut small = cfg;
    small.duration = Duration::from_secs(2);
    small.warmup = Duration::from_secs(1);
    assert!(run_effective(&t, &e, &EffectiveChaos::default(), &small).is_ok());
    assert!(
        run_with_limits(
            &t,
            &e,
            &EffectiveChaos::default(),
            &small,
            &Limits::server()
        )
        .is_ok()
    );
}

#[test]
fn default_limits_equal_the_old_constants() {
    let d = Limits::default();
    assert_eq!(d.max_hop_records.to_bits(), 20_000_000.0_f64.to_bits());
    assert_eq!(d.max_sim_hops.to_bits(), 50_000_000.0_f64.to_bits());
    assert!(Limits::server().max_hop_records < d.max_hop_records);
}
