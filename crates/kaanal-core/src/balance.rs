//! Replicas and balancing: how many copies a node has and how traffic entering it picks one.
//!
//! The config side only: [`Balance`] and the replica cap. The pick itself is a pure function
//! next to it (added with the executor change), so the simulator and the mocks choose alike.

use rand::Rng;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::exec::draw_rng;
use crate::rng::HopRng;

/// Most replicas one node may have.
pub const MAX_REPLICAS: u16 = 64;

/// How traffic entering a node picks a replica. A node sets one for traffic entering it; a calling
/// edge may override it (an entry node has no calling edge, so the node's own is used).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Balance {
    /// Each request takes the next live replica in turn.
    #[default]
    RoundRobin,
    /// Each request takes a uniformly random live replica.
    Random,
    /// A random live replica, weighted: one weight per replica (a 0 weight gets none).
    Weighted(Vec<u32>),
}

impl<'de> Deserialize<'de> for Balance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        const EXPECTED: &str = "expected `round-robin`, `random` or `{ weighted: [3, 1, 1] }`";
        /// **Private.** `{ weighted: [...] }`; unknown keys rejected.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WeightedRaw {
            weighted: Vec<u32>,
        }
        /// **Private.** The two YAML shapes of a balance.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Weighted(WeightedRaw),
        }
        match Raw::deserialize(d).map_err(|_| D::Error::custom(EXPECTED))? {
            Raw::Name(n) => match n.as_str() {
                "round-robin" => Ok(Balance::RoundRobin),
                "random" => Ok(Balance::Random),
                "weighted" => Err(D::Error::custom(
                    "weighted needs weights: write `{ weighted: [3, 1, 1] }` (one per replica)",
                )),
                _ => Err(D::Error::custom(EXPECTED)),
            },
            Raw::Weighted(w) => Ok(Balance::Weighted(w.weighted)),
        }
    }
}

/// The live replica a request takes, or `None` when every replica is down. Pure: the same inputs
/// give the same answer, in the simulator and in the mocks (the replica index never enters a seed).
///
/// `up[i]` is false for a replica that is down (a missing entry counts as up). `n` is the
/// endpoint's request counter and `offset` a per-(endpoint, node) shift, so two endpoints do not
/// both start at replica 0. Round-robin takes the `(n + offset) % live`-th live replica. Random and
/// weighted draw from `path` and `node` under the tag `replica`, so no existing draw moves;
/// weighted ignores a down replica's weight, and falls back to uniform when the live replicas all
/// weigh 0 (a load balancer with no healthy preferred backend still serves).
pub fn pick_replica(
    balance: &Balance,
    replicas: u16,
    up: &[bool],
    n: u64,
    offset: u64,
    path: u64,
    node: &str,
) -> Option<u16> {
    let live: Vec<u16> = (0..replicas)
        .filter(|i| up.get(usize::from(*i)).copied().unwrap_or(true))
        .collect();
    if live.is_empty() {
        return None;
    }
    let count = live.len() as u64;
    let uniform = |rng: &mut HopRng| live[usize::try_from(rng.random_range(0..count)).unwrap_or(0)];
    Some(match balance {
        Balance::RoundRobin => live[usize::try_from(n.wrapping_add(offset) % count).unwrap_or(0)],
        Balance::Random => uniform(&mut draw_rng(path, node, b"replica")),
        Balance::Weighted(w) => {
            let weight = |i: u16| u64::from(w.get(usize::from(i)).copied().unwrap_or(0));
            let total: u64 = live.iter().map(|i| weight(*i)).sum();
            let mut rng = draw_rng(path, node, b"replica");
            if total == 0 {
                return Some(uniform(&mut rng));
            }
            let mut t = rng.random_range(0..total);
            *live
                .iter()
                .find(|i| {
                    let w = weight(**i);
                    let hit = t < w;
                    t = t.saturating_sub(w);
                    hit
                })
                .unwrap_or(&live[live.len() - 1])
        }
    })
}
