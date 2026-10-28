//! Studio's simulation API behind the control routes: turning a traffic profile into simulator
//! input ([`profile`]) and sampling a body ([`sample`]). Runs and their store come next to them.

pub(crate) mod profile;
pub(crate) mod run;
pub(crate) mod sample;
pub(crate) mod store;

pub use store::{RunRecord, SimStore};

/// How many `/sample` and `/openapi/skeleton` jobs may run at once.
pub const WORK_PERMITS: usize = 2;
