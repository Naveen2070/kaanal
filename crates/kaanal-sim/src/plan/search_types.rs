//! What the cheapest search (Step 9c-2) takes and gives, beside the plan of 9c-1.

use std::time::Duration;

use serde::Serialize;

use super::types::Plan;

/// The label every answer of the search carries (spec anti-goal: never "cheapest" without it).
pub const CHEAPEST_NOTE: &str = "cheapest found within the limit, not proven cheapest";

/// The bounds of one search.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Checked between runs, never inside one: the overshoot is at most one run.
    pub time_limit: Duration,
    /// The most runs, the walk's included.
    pub run_cap: u32,
}

impl Default for SearchOptions {
    /// 150 s and 200 runs (spec defaults).
    fn default() -> Self {
        SearchOptions {
            time_limit: Duration::from_secs(150),
            run_cap: 200,
        }
    }
}

/// Why the search stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// No cheaper candidate holds under the pruning rule (or the walk found no holding design).
    Exhausted,
    /// `run_cap` runs used.
    RunCap,
    /// The time limit passed before the next run.
    TimeLimit,
}

/// What the record says about one search.
#[derive(Debug, Clone, Serialize)]
pub struct SearchInfo {
    /// Why it stopped.
    pub stop: StopReason,
    /// Runs used, the walk's included.
    pub runs: u32,
    /// Time used, by the injected clock.
    pub elapsed_ms: u64,
    /// The time limit.
    pub limit_ms: u64,
    /// The run cap.
    pub run_cap: u32,
    /// True when the design in force held the target before any move.
    pub design_in_force_holds: bool,
    /// Candidates refused for size, by move.
    pub notes: Vec<String>,
}

/// The search's answer: the plan (steps, verdict, prices as in 9c-1) and how the search ran.
#[derive(Debug, Clone, Serialize)]
pub struct Searched {
    /// The cheapest holding design found, or the walk's plan when none holds.
    pub plan: Plan,
    /// The record of the search.
    pub info: SearchInfo,
}

/// The time of the main search and of each of `sections` provider sections: half to the main
/// search when there are sections (all of it otherwise), the rest split equally. Unused time is not
/// carried over.
#[must_use]
pub fn split_time(limit: Duration, sections: usize) -> (Duration, Vec<Duration>) {
    if sections == 0 {
        return (limit, Vec::new());
    }
    let main = limit / 2;
    let each = limit.saturating_sub(main) / u32::try_from(sections).unwrap_or(u32::MAX);
    (main, vec![each; sections])
}
