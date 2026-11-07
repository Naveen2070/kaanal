//! The run summary's account of retries and circuit breakers (step 7f): per wire, how many calls
//! carried a retry scope and how they ended (the funnel the Studio draws), and per breaker the state
//! it was in over the window (the timeline). Both come from counters and a log the engine keeps
//! beside the per-request records, so a run without retries or breakers pays nothing.

use std::time::Duration;

use kaanal_core::Topology;
use kaanal_core::replay::{BreakerChange, BreakerState};
use serde::Serialize;

use crate::SimReport;

/// What the retry scopes of one wire did over the reporting window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrySummary {
    /// Calling node.
    pub from: String,
    /// Called node.
    pub to: String,
    /// Calls that carried a retry scope (the first tries).
    pub calls: u64,
    /// Calls that made at least one retry.
    pub retried: u64,
    /// Retry attempts, all calls (a call retried twice adds 2).
    pub retries: u64,
    /// Retried calls that ended well.
    pub recovered: u64,
    /// Retried calls that did not (`retried - recovered`): the calls that gave up.
    pub gave_up: u64,
    /// Calls whose last attempt failed, retried or not.
    pub failed: u64,
}

/// One breaker state change, `t_ms` after the window opened.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BreakerChangeSummary {
    /// Milliseconds since the reporting window began.
    pub t_ms: f64,
    /// `closed`, `half_open` or `open`.
    pub state: &'static str,
}

/// One breaker (a wire and the caller replica holding it) over the reporting window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BreakerSummary {
    /// Calling node.
    pub from: String,
    /// Called node.
    pub to: String,
    /// The caller replica holding this breaker.
    pub replica: u16,
    /// The state when the window began (a breaker starts closed; one that opened during warmup is open).
    pub state_at_start: &'static str,
    /// State changes inside the window, in order.
    pub changes: Vec<BreakerChangeSummary>,
    /// Milliseconds the breaker spent open inside the window.
    pub open_ms: f64,
}

/// Nanoseconds as milliseconds.
fn ms(ns: u64) -> f64 {
    Duration::from_nanos(ns).as_secs_f64() * 1000.0
}

fn name(s: BreakerState) -> &'static str {
    match s {
        BreakerState::Closed => "closed",
        BreakerState::HalfOpen => "half_open",
        BreakerState::Open => "open",
    }
}

/// One entry per wire that carried at least one retry scope in the window, in edge order.
pub(super) fn retry_summaries(topo: &Topology, report: &SimReport) -> Vec<RetrySummary> {
    report
        .wires
        .iter()
        .zip(&topo.edges)
        .filter(|(w, _)| w.calls > 0)
        .map(|(w, e)| RetrySummary {
            from: topo.nodes[e.from].name.clone(),
            to: topo.nodes[e.to].name.clone(),
            calls: w.calls,
            retried: w.retried,
            retries: w.retries,
            recovered: w.recovered,
            gave_up: w.retried.saturating_sub(w.recovered),
            failed: w.failed,
        })
        .collect()
}

/// One entry per breaker that changed state at any point of the run, by edge then replica.
pub(super) fn breaker_summaries(
    topo: &Topology,
    report: &SimReport,
    warmup: Duration,
    end: Duration,
) -> Vec<BreakerSummary> {
    let ns = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
    let (from_ns, end_ns) = (ns(warmup), ns(end));
    let mut keys: Vec<(u16, u16)> = report
        .breaker_changes
        .iter()
        .map(|c| (c.edge, c.replica))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .filter_map(|(edge, replica)| {
            let log: Vec<&BreakerChange> = report
                .breaker_changes
                .iter()
                .filter(|c| c.edge == edge && c.replica == replica)
                .collect();
            let e = topo.edges.get(usize::from(edge))?;
            // the state when the window began: the last change at or before it
            let start = log
                .iter()
                .take_while(|c| c.at <= from_ns)
                .last()
                .map_or(BreakerState::Closed, |c| c.state);
            let inside: Vec<&&BreakerChange> = log
                .iter()
                .filter(|c| c.at > from_ns && c.at <= end_ns)
                .collect();
            // time open: walk the segments between changes
            let (mut open_ns, mut at, mut state) = (0u64, from_ns, start);
            for c in &inside {
                if state == BreakerState::Open {
                    open_ns += c.at - at;
                }
                (at, state) = (c.at, c.state);
            }
            if state == BreakerState::Open {
                open_ns += end_ns.saturating_sub(at);
            }
            Some(BreakerSummary {
                from: topo.nodes[e.from].name.clone(),
                to: topo.nodes[e.to].name.clone(),
                replica,
                state_at_start: name(start),
                changes: inside
                    .iter()
                    .map(|c| BreakerChangeSummary {
                        t_ms: ms(c.at - from_ns),
                        state: name(c.state),
                    })
                    .collect(),
                open_ms: ms(open_ns),
            })
        })
        .collect()
}
