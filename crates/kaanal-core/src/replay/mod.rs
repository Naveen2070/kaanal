//! The replay machine both drivers share (B1, rule R2.5/R2.10).
//!
//! A request's [`crate::Plan`] is walked against per-node slot limits ([`Gate`]) and rate
//! limits ([`Bucket`]). Everything here is pure: time is a `u64` of nanoseconds that the
//! caller supplies, and nothing sleeps, reads a clock or does IO. The simulator drives it with
//! virtual time, live mode with real time.

mod bucket;
mod engine;
mod gate;

pub use bucket::Bucket;
pub use engine::{Engine, EngineError, FailCause, Finished, FinishedHop, NodeCounters};
pub use gate::{Admit, Gate, Waiter};
