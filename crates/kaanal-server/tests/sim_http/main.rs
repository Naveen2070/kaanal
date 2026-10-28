//! Integration tests for the simulation API (`/__kaanal/sim/*`, `/__kaanal/sample`).

#[path = "../draft_http/common.rs"]
#[expect(
    dead_code,
    reason = "shared with draft_http, which uses helpers this crate does not"
)]
mod common;

mod run;
mod sample;
