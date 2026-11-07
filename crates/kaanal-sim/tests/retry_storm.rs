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

/// One 40 s run at `rate` a second over the gateway-to-service edge `edge`.
fn simulate(
    edge: &str,
    rate: f64,
) -> (
    Topology,
    kaanal_core::Endpoints,
    SimConfig,
    kaanal_sim::SimReport,
) {
    let y = format!(
        "version: 1
nodes:
  gw: {{ type: gateway }}
  app: {{ type: service, concurrency: {{ limit: 2, queue: 200 }}, timeout: 50ms, proc: {{ mean: 10ms }} }}
edges:
  - {{ from: gw, to: app{edge} }}
"
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
    (t, e, cfg, rep)
}

/// `(failed, short-circuited)`: the shares of requests that failed, and that an open breaker refused
/// without sending (a subset of the failed ones).
#[allow(clippy::cast_precision_loss)]
fn shares(edge: &str, rate: f64) -> (f64, f64) {
    let (_, _, _, rep) = simulate(edge, rate);
    let bad = rep
        .requests
        .iter()
        .filter(|r| !matches!(r.result, ChainResult::Ok))
        .count();
    let rejected = rep
        .requests
        .iter()
        .filter(|r| r.cause == Some(kaanal_sim::Cause::Breaker))
        .count();
    let n = rep.requests.len() as f64;
    (bad as f64 / n, rejected as f64 / n)
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

#[test]
fn the_summary_counts_what_the_breaker_refused_and_warns() {
    let (t, e, cfg, rep) = simulate(BREAKER, 120.0);
    let refused = rep
        .requests
        .iter()
        .filter(|r| r.cause == Some(kaanal_sim::Cause::Breaker))
        .count() as u64;
    let sum = kaanal_sim::summarize(&t, &e, &cfg, &rep);
    let app = sum.nodes.iter().find(|n| n.name == "app").unwrap();
    // an attempt of a retried call can be refused too, so the counter is at least the refused requests
    assert!(
        refused > 0 && app.short_circuited >= refused,
        "{} vs {refused}",
        app.short_circuited
    );
    assert!(
        sum.warnings.iter().any(|w| w.contains("circuit breaker")),
        "{:?}",
        sum.warnings
    );
    // no breaker, no counter
    let (t, e, cfg, rep) = simulate("", 120.0);
    let sum = kaanal_sim::summarize(&t, &e, &cfg, &rep);
    assert!(sum.nodes.iter().all(|n| n.short_circuited == 0));
}

#[test]
fn the_record_lists_each_retry_with_its_number_and_start() {
    let (t, _, _, rep) = simulate(CONTINUE, 150.0);
    let app = t.id("app").unwrap();
    assert!(
        !rep.attempts.is_empty(),
        "a loaded system with retries retries"
    );
    for (&(stream, n), list) in &rep.attempts {
        let r = rep
            .requests
            .iter()
            .find(|r| r.stream == stream && r.n == n)
            .expect("a retried request is a reported one");
        assert!(list.len() <= 2, "at most two retries of three tries");
        for (i, &(at, num, of, node)) in list.iter().enumerate() {
            assert_eq!((usize::from(num), of, node), (i + 2, 3, app));
            assert!(
                at <= r.latency,
                "a retry starts before the request is answered"
            );
        }
        assert!(list.windows(2).all(|w| w[0].0 <= w[1].0));
    }
    // no retry configured: none recorded
    let (_, _, _, plain) = simulate("", 150.0);
    assert!(plain.attempts.is_empty());
}

#[test]
fn the_summary_counts_calls_retries_and_how_they_ended_per_wire() {
    let (t, e, cfg, rep) = simulate(CONTINUE, 150.0);
    let sum = kaanal_sim::summarize(&t, &e, &cfg, &rep);
    assert_eq!(sum.retries.len(), 1, "one wire carries a retry scope");
    let w = &sum.retries[0];
    assert_eq!((w.from.as_str(), w.to.as_str()), ("gw", "app"));
    // the funnel narrows: every call is a first try, some retried, some of those recovered
    assert!(w.calls > 0 && w.retried > 0, "{w:?}");
    assert!(w.recovered <= w.retried && w.retried <= w.calls, "{w:?}");
    assert_eq!(w.gave_up, w.retried - w.recovered);
    assert!(w.failed <= w.calls, "{w:?}");
    // two retries at most per call (three tries), and every retry the record lists was counted
    let listed: u64 = rep.attempts.values().map(|l| l.len() as u64).sum();
    assert!(
        w.retries >= listed && w.retries <= 2 * w.retried,
        "{w:?} vs {listed}"
    );
    // at 75% load with abandoned work left running the collapse is near total (see above)
    assert!(w.failed * 10 > w.calls * 9, "{w:?}");
    // no retry configured: nothing to report
    let (t, e, cfg, rep) = simulate("", 150.0);
    assert_eq!(kaanal_sim::summarize(&t, &e, &cfg, &rep).retries.len(), 0);
}

#[test]
fn the_summary_gives_each_breaker_a_timeline_inside_the_window() {
    let (t, e, cfg, rep) = simulate(BREAKER, 120.0);
    let sum = kaanal_sim::summarize(&t, &e, &cfg, &rep);
    assert_eq!(sum.breakers.len(), 1);
    let b = &sum.breakers[0];
    assert_eq!(
        (b.from.as_str(), b.to.as_str(), b.replica),
        ("gw", "app", 0)
    );
    assert!(!b.changes.is_empty(), "the breaker opened during the run");
    let window_ms = sum.window_s * 1000.0;
    assert!(
        b.changes
            .iter()
            .all(|c| c.t_ms > 0.0 && c.t_ms <= window_ms)
    );
    assert!(b.changes.windows(2).all(|w| w[0].t_ms <= w[1].t_ms));
    // only legal moves: closed -> open, open -> half open, half open -> open or closed
    let mut state = b.state_at_start;
    for c in &b.changes {
        let ok = matches!(
            (state, c.state),
            ("closed", "open") | ("open", "half_open") | ("half_open", "open" | "closed")
        );
        assert!(ok, "{state} -> {}", c.state);
        state = c.state;
    }
    assert!(b.open_ms > 0.0 && b.open_ms <= window_ms, "{}", b.open_ms);
    // it was open for at least one full open_for (2 s) of the window unless the window cut it
    assert!(b.open_ms >= 1900.0, "{}", b.open_ms);
    // no breaker configured: none reported
    let (t, e, cfg, rep) = simulate(CONTINUE, 120.0);
    assert_eq!(kaanal_sim::summarize(&t, &e, &cfg, &rep).breakers.len(), 0);
}
