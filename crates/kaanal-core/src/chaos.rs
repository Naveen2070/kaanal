use std::collections::HashMap;
use std::time::Duration;

use crate::topology::NodeId;

#[derive(Debug, Clone, PartialEq)]
pub struct NodeChaos {
    pub latency_add: Duration,
    /// 0.0..=1.0
    pub error_rate: f64,
    pub error_status: u16,
    pub down: bool,
}

impl Default for NodeChaos {
    fn default() -> Self {
        NodeChaos {
            latency_add: Duration::ZERO,
            error_rate: 0.0,
            error_status: 503,
            down: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChaosState {
    pub nodes: HashMap<NodeId, NodeChaos>,
}
