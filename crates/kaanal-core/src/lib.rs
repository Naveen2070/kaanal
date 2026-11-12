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

pub mod balance;
pub mod chain;
pub mod chaos;
pub mod config;
pub mod cost;
pub mod diagnostics;
pub mod dist;
pub mod exec;
pub mod instances;
pub mod pin;
pub mod presets;
pub mod prices;
mod prices_data;
pub mod replay;
pub mod rng;
pub mod scenario;
pub mod schema;
pub mod spec_skeleton;
pub mod splice;
pub mod topology;
pub mod units;
pub mod visits;

pub use balance::{Balance, MAX_REPLICAS, pick_replica};
pub use chain::{
    CollectionOp, Endpoint, Endpoints, EndpointsInput, JWKS_KEY, LOGIN_KEY, Responder, RouteSpec,
    collection_op, compile, resolve_error, skeleton,
};
pub use chaos::{
    ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, ErrorKind, FailKind, Fault, HANG_CAP, KeyMap,
    MAX_FAULTS, MAX_LATENCY_ADD, NodeChaos, Resolved, compile_layers,
};
pub use config::{
    BreakerConfig, Budget, ChaosEntry, Config, ConfigError, Determinism, EdgeRef, EndpointConfig,
    EndpointsConfig, Hold, JourneyConfig, NodeKind, RetryConfig, ScenarioConfig, SeedSource,
    TrafficConfig, check_profile, parse_scenario, preprocess, strip_bom, valid_locale_tag,
};
pub use diagnostics::{Located, PathSeg};
pub use dist::Dist;
pub use exec::{
    BreakerRef, ChainOutcome, ChainRequest, ChainResult, DYN_BASE, Hop, HopKind, NO_EDGE, Op,
    PickKind, Pipe, Plan, PlanItem, RetryBudget, ScopedFail, Xfer, execute, sample_plan,
};
pub use pin::{
    EffectivePins, EmptyOk, Outcome, PinEntry, PinEntryConfig, PinLayer, PinSource, PinState,
    ResolvedPin, StateOverride, StateOverrideConfig, StateOverrides, resolve, validate_entry,
};
pub use replay::{
    Engine, EngineError, FailCause, Finished, FinishedAttempt, FinishedHold, FinishedHop,
    NodeCounters,
};
pub use rng::HopRng;
pub use scenario::{
    ChaosStack, MAX_ENTRIES, MAX_SCENARIOS, Scenario, ScenarioError, ScenarioSet, Source,
    endpoint_key, resolve_scenario, valid_name, validate_scenario_shape,
};
pub use schema::config_schema;
pub use spec_skeleton::skeleton_openapi;
pub use topology::{
    Admission, CacheModel, Edge, EdgeId, Node, NodeId, RateLimit, Topology, TopologyError,
};
pub use visits::{expected_visits, visits_per_s};

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
