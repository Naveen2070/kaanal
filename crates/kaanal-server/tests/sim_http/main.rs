//! Integration tests for the simulation API (`/__kaanal/sim/*`, `/__kaanal/sample`).

#[path = "../draft_http/common.rs"]
#[expect(
    dead_code,
    reason = "shared with draft_http, which uses helpers this crate does not"
)]
mod common;

mod draft_run;
mod log;
mod run;
mod sample;
mod skeleton;
mod sweep;
mod sweep_chaos;
mod sweep_report;
