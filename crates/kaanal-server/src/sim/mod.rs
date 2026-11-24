//! Studio's simulation API behind the control routes: turning a traffic profile into simulator
//! input ([`profile`]) and sampling a body ([`sample`]). Runs and their store come next to them,
//! and a finished sweep's report ([`report`]).

pub(crate) mod draft_run;
pub(crate) mod log;
pub(crate) mod plan_draft;
pub(crate) mod plan_search;
pub(crate) mod profile;
pub(crate) mod report;
pub(crate) mod run;
pub(crate) mod sample;
pub(crate) mod store;

pub use store::{RunRecord, SimStore};

/// How many `/sample`, `/openapi/skeleton` and draft-build (`source: "draft"` run) jobs may run at once.
pub const WORK_PERMITS: usize = 2;
