//! Live chaos state: the per-node failure knobs flipped at runtime via the control API.
//!
//! Nothing here comes from `kaanal.yaml`; this is runtime state only. Written by
//! `PUT/DELETE /__kaanal/chaos...`, read by the chain executor on every request.

use std::collections::HashMap;
use std::time::Duration;

use crate::topology::NodeId;

/// Longest extra latency chaos may add to a node (5 min). A typo like 1e12 ms would
/// otherwise hang every request (live) or overflow the virtual clock (simulator).
pub const MAX_LATENCY_ADD: Duration = Duration::from_secs(300);

/// Chaos settings for one node.
///
/// Every driver checks [`NodeChaos::validate`] before anything lands here (rule R6.9);
/// the executor trusts the values.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeChaos {
    /// Extra latency added to every visit of the node.
    pub latency_add: Duration,
    /// Probability of failing a visit, 0.0..=1.0.
    pub error_rate: f64,
    /// HTTP status a failed visit returns.
    pub error_status: u16,
    /// Hard outage: every visit fails with 503, regardless of `error_rate`.
    pub down: bool,
}

impl Default for NodeChaos {
    /// All-off defaults. An all-default entry is inert: it changes no timings and
    /// no outcomes, which the executor's determinism tests rely on.
    fn default() -> Self {
        NodeChaos {
            latency_add: Duration::ZERO,
            error_rate: 0.0,
            error_status: 503,
            down: false,
        }
    }
}

impl NodeChaos {
    /// The one set of chaos limits, shared by the control API and the simulator (rule R6.9):
    /// `latency_add` at most [`MAX_LATENCY_ADD`], `error_rate` within 0..=1, `error_status`
    /// within 500–504 (4xx is for pins and cycle states, not random chaos).
    ///
    /// # Errors
    /// A message naming the first field out of range.
    pub fn validate(&self) -> Result<(), String> {
        if self.latency_add > MAX_LATENCY_ADD {
            return Err(format!(
                "latency_add must be at most {} ms (5 min)",
                MAX_LATENCY_ADD.as_millis()
            ));
        }
        if !(0.0..=1.0).contains(&self.error_rate) {
            return Err("error_rate must be between 0 and 1".into());
        }
        if !(500..=504).contains(&self.error_status) {
            return Err("error_status must be 500-504".into());
        }
        Ok(())
    }
}

/// Chaos settings of every node, keyed by node. An absent node has no chaos.
pub type ChaosState = HashMap<NodeId, NodeChaos>;
