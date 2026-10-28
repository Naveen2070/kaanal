//! Running the simulator for the control API. [`execute_run`] is the only place the server calls
//! `kaanal_sim` to run: moving the simulator into its own process later (see
//! `internal/docs/suggestions/2026-10-28-process-isolation.md`) replaces its body and nothing else.

use std::sync::Arc;

use kaanal_core::EffectiveChaos;
use kaanal_sim::{Limits, SimConfig, Summary, run_with_limits, summarize};

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

/// Runs the simulation and folds the report into a summary; the hop records are dropped here.
///
/// # Errors
/// The simulator's message.
pub(crate) fn execute_run(job: &Job) -> Result<Summary, String> {
    let report = run_with_limits(
        &job.g.topo,
        &job.g.endpoints,
        &job.chaos,
        &job.cfg,
        &Limits::server(),
    )?;
    Ok(summarize(&job.g.topo, &job.g.endpoints, &job.cfg, &report))
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
    pub(crate) fn finish(mut self, result: Result<Summary, String>) {
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
