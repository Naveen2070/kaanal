//! Running the simulator for the control API. [`execute_run`] is the only place the server calls
//! `kaanal_sim` to run: moving the simulator into its own process later (see
//! `internal/docs/suggestions/2026-10-28-process-isolation.md`) replaces its body and nothing else.

use std::collections::BTreeMap;
use std::sync::Arc;

use kaanal_core::EffectiveChaos;
use kaanal_core::cost::{CostReport, egress_flows, estimate_with_egress};
use kaanal_sim::{Limits, SimConfig, Summary, run_with_limits, summarize};

use super::log::{LogSample, sample};
use crate::generation::Generation;
use crate::state::AppState;

/// Everything one run needs, owned so it can move to a blocking thread.
pub(crate) struct Job {
    /// The system to simulate.
    pub g: Arc<Generation>,
    /// The chaos in force when the run started (live, or one named scenario).
    pub chaos: EffectiveChaos,
    /// The streams, duration, warmup and seed.
    pub cfg: SimConfig,
}

/// Checks `job` against the server caps without running it, so a request that is too big is a
/// 400 and not a failed run.
///
/// # Errors
/// The simulator's message (rate range, unknown endpoint, workload too large, ...).
pub(crate) fn check(job: &Job) -> Result<(), String> {
    kaanal_sim::check_with_limits(&job.g.topo, &job.g.endpoints, &job.cfg, &Limits::server())
}

/// Runs the simulation and folds the report into a summary and a log sample; the other hop
/// records are dropped here.
///
/// # Errors
/// The simulator's message.
pub(crate) fn execute_run(job: &Job) -> Result<(Summary, LogSample, CostReport), String> {
    let report = run_with_limits(
        &job.g.topo,
        &job.g.endpoints,
        &job.chaos,
        &job.cfg,
        &Limits::server(),
    )?;
    let log = sample(&report, &job.g.topo, &job.g.endpoints, &job.cfg);
    let summary = summarize(&job.g.topo, &job.g.endpoints, &job.cfg, &report);
    // an object store's requests are priced at what this run sent it
    let visits: BTreeMap<String, f64> = summary
        .nodes
        .iter()
        .map(|n| (n.name.clone(), n.visits_per_s))
        .collect();
    // and what its streams send out to the internet: rate x response size, by the cloud of the entry node
    let flows = egress_flows(
        &job.g.config,
        &job.g.topo,
        &job.g.endpoints,
        job.cfg.streams.iter().map(|s| {
            #[allow(clippy::cast_precision_loss)]
            (s.endpoint_key.as_str(), s.rate_per_s * s.resp_bytes as f64)
        }),
    );
    let cost = estimate_with_egress(&job.g.config, &visits, &flows);
    Ok((summary, log, cost))
}

/// Frees the run slot with a failure if the run never reported: a panic in the simulator, or a
/// closure the blocking pool dropped without running (it owns the state, so `Drop` still runs).
pub(crate) struct Slot {
    st: Arc<AppState>,
    id: String,
    done: bool,
}

impl Slot {
    /// Guards run `id`.
    pub(crate) fn new(st: Arc<AppState>, id: String) -> Self {
        Slot {
            st,
            id,
            done: false,
        }
    }

    /// Records the result and releases the slot.
    pub(crate) fn finish(mut self, result: Result<(Summary, LogSample, CostReport), String>) {
        self.done = true;
        self.st.sims.finish(&self.id, result);
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if !self.done {
            self.st
                .sims
                .finish(&self.id, Err("the run stopped unexpectedly".into()));
        }
    }
}
