//! Pure logic for the Kaanal infrastructure emulator.
//!
//! This crate owns everything that must stay deterministic and testable: `kaanal.yaml`
//! parsing ([`Config`]), topology graph validation ([`Topology`]), latency
//! distributions ([`Dist`]), live chaos state ([`ChaosState`]) and the call-chain
//! executor ([`execute`]).
//!
//! It is deliberately pure: no tokio, no axum, no IO, no global state. It computes
//! outcomes; it never sleeps. The server sleeps the durations it returns (live mode)
//! or advances virtual time (the simulator), so both drivers share one cost model.

#![warn(missing_docs)]

pub mod chaos;
pub mod config;
pub mod dist;
pub mod exec;
pub mod topology;

pub use chaos::{ChaosState, NodeChaos};
pub use config::{Config, ConfigError, NodeKind};
pub use dist::Dist;
pub use exec::{ChainOutcome, ChainRequest, ChainResult, Hop, HopKind, Op, execute};
pub use topology::{Edge, Node, NodeId, Topology, TopologyError};

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
