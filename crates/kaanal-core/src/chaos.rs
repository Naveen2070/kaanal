//! Live chaos state: the per-node failure knobs flipped at runtime via the control API.
//!
//! Nothing here comes from `kaanal.yaml`; this is runtime state only. Written by
//! `PUT/DELETE /__kaanal/chaos...`, read by the chain executor on every request.

use std::collections::HashMap;
use std::time::Duration;

use crate::topology::NodeId;

/// Chaos settings for one node.
///
/// The control API validates ranges before anything lands here
/// (`error_rate` within 0.0–1.0, `error_status` within 500–504);
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

/// Chaos settings of every node, keyed by node. An absent node has no chaos.
pub type ChaosState = HashMap<NodeId, NodeChaos>;
