//! A retry storm, the metastable failure of Bronson et al., "Metastable Failures in Distributed
//! Systems" (`HotOS` 2021): load-dependent timeouts plus retries that keep the callee's work running
//! (`abandon: continue`) feed on themselves, until almost every request fails, at a load the same
//! system carries without retries. A retry budget (gRPC A6) breaks the loop.
//! Two slots at 10 ms mean service: capacity 200 requests a second; 50 ms timeout at the service.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{ChainResult, ChaosState, Config, Topology};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

/// The share of requests that failed over a run at `rate` a second, for the gateway-to-service edge `edge`.
fn failed_share(edge: &str, rate: f64) -> f64 {
    shares(edge, rate).0
}

/// `(failed, short-circuited)`: the shares of requests that failed, and that an open breaker refused
/// without sending (a subset of the failed ones).
#[allow(clippy::cast_precision_loss)]
fn shares(edge: &str, rate: f64) -> (f64, f64) {
    let y = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, concurrency: {{ limit: 2, queue: 200 }}, timeout: 50ms, proc: {{ mean: 10ms }} }}\nedges:\n  - {{ from: gw, to: app{edge} }}\n"
    );
    let t = Topology::build(&Config::from_yaml(&y).unwrap()).unwrap();
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: rate,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(40),
        warmup: Duration::from_secs(5),
        seed: 7,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let bad = rep
        .requests
        .iter()
        .filter(|r| !matches!(r.result, ChainResult::Ok))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let share = bad as f64 / rep.requests.len() as f64;
    let rejected = rep
        .requests
        .iter()
        .filter(|r| r.cause == Some(kaanal_sim::Cause::Breaker))
        .count();
    (share, rejected as f64 / rep.requests.len() as f64)
}

const RETRY: &str = ", retry: { max_attempts: 3 }";
const CONTINUE: &str = ", retry: { max_attempts: 3 }, abandon: continue";
const BUDGET: &str =
    ", retry: { max_attempts: 3, budget: { max_tokens: 10, token_ratio: 0.1 } }, abandon: continue";

#[test]
fn retries_that_leave_abandoned_work_running_collapse_a_system_that_a_budget_saves() {
    // at 60% load the system is healthy: 2.6% of requests time out with no retry at all
    let (none, retry, collapse, budget) = (
        failed_share("", 120.0),
        failed_share(RETRY, 120.0),
        failed_share(CONTINUE, 120.0),
        failed_share(BUDGET, 120.0),
    );
    assert!(none < 0.06, "no retry: {none}");
    // retrying a timed-out call whose work was dropped helps...
    assert!(retry < none, "retry with cancel: {retry} vs {none}");
    // ...but each retry added to the load of work that nobody waits for: most requests fail
    assert!(collapse > 0.5, "retry with continue: {collapse}");
    // the budget stops the amplification: back near the no-retry rate
    assert!(budget < 0.06, "retry with continue and a budget: {budget}");
}

#[test]
fn near_capacity_the_collapse_is_total_and_the_budget_still_halves_the_damage() {
    // at 75% load
    let (continuing, budget) = (failed_share(CONTINUE, 150.0), failed_share(BUDGET, 150.0));
    assert!(continuing > 0.9, "retry with continue: {continuing}");
    assert!(budget < continuing / 2.0, "with a budget: {budget}");
}

const BREAKER: &str = ", retry: { max_attempts: 3 }, abandon: continue, breaker: { window: 50, min_calls: 50, open_for: 2s, half_open_calls: 10 }";

#[test]
fn a_breaker_lets_the_callee_recover_by_failing_fast_instead() {
    // The same collapse as above, with a breaker on the edge: once half the last 50 calls fail it opens,
    // the retries and their abandoned work stop reaching the callee, the queue drains, and the probes
    // find it healthy. Most of what still fails is the breaker's own fast 503, not the callee timing out.
    for (rate, collapsed) in [(120.0, 0.5), (150.0, 0.9)] {
        let (failed, short) = shares(BREAKER, rate);
        assert!(
            failed < collapsed,
            "rate {rate}: {failed} vs a collapse above {collapsed}"
        );
        let at_callee = failed - short;
        assert!(
            at_callee < 0.06,
            "rate {rate}: {at_callee} of requests failed at the callee"
        );
        assert!(short > 0.0, "rate {rate}: the breaker never opened");
    }
}
