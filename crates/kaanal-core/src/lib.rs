//! Pure logic for the Kaanal infrastructure emulator.
//!
//! This crate owns everything that must stay deterministic and testable: `kaanal.yaml`
//! parsing ([`Config`]), topology graph validation ([`Topology`]), flavor presets
//! ([`presets`]), the editor JSON Schema ([`config_schema`]), latency distributions
//! ([`Dist`]), live chaos state ([`ChaosState`]) and the call-chain executor
//! ([`execute`]: a timeline and a replayable [`Plan`] per request), `endpoints:` parsing
//! ([`config::parse_endpoints`]) and compilation ([`compile`], including an auth node's built-in login and JWKS routes),
//! with error positions for config problems ([`diagnostics`]).
//!
//! It is deliberately pure: no tokio, no axum, no IO, no global state. It computes
//! outcomes; it never sleeps. The server runs the plans it returns through the replay engine in real time (live mode)
//! or advances virtual time (the simulator), so both drivers share one cost model.
//! The replay machine they share ([`replay`]: token buckets, slot gates and the step-driven engine) is pure too.

#![warn(missing_docs)]

pub mod chain;
pub mod chaos;
pub mod config;
pub mod diagnostics;
pub mod dist;
pub mod exec;
pub mod presets;
pub mod replay;
pub mod scenario;
pub mod schema;
pub mod topology;

pub use chain::{
    Endpoint, Endpoints, EndpointsInput, JWKS_KEY, LOGIN_KEY, Responder, RouteSpec, compile,
    resolve_error, skeleton,
};
pub use chaos::{
    ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, ErrorKind, FailKind, Fault, HANG_CAP,
    MAX_FAULTS, MAX_LATENCY_ADD, NodeChaos, Resolved, compile_layers, merge,
};
pub use config::{
    ChaosEntry, Config, ConfigError, Determinism, EndpointsConfig, NodeKind, ScenarioConfig,
    SeedSource, parse_scenario,
};
pub use diagnostics::{Located, PathSeg};
pub use dist::Dist;
pub use exec::{
    ChainOutcome, ChainRequest, ChainResult, Hop, HopKind, Op, Plan, PlanItem, execute, sample_plan,
};
pub use replay::{Engine, EngineError, Finished, FinishedHop, NodeCounters};
pub use scenario::{
    ChaosStack, MAX_ENTRIES, MAX_SCENARIOS, Scenario, ScenarioError, ScenarioSet, Source,
    endpoint_key, resolve_scenario, valid_name,
};
pub use schema::config_schema;
pub use topology::{Admission, CacheModel, Edge, Node, NodeId, RateLimit, Topology, TopologyError};

/// FNV-1a 64 over the parts, with a separator byte between parts.
/// Stable across platforms and Rust versions (std hashers are not).
pub fn stable_hash(parts: &[&[u8]]) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in *part {
            h ^= u64::from(*b);
            h = h.wrapping_mul(PRIME);
        }
        h ^= 0xff;
        h = h.wrapping_mul(PRIME);
    }
    h
}
