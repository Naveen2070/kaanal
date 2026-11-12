//! Your own instance types (`instances:` at the top of `kaanal.yaml`, RULES 113): a machine that is
//! not in the cloud table (a self-hosted server, an on-premises box, another provider's size)
//! described once and used by name, `instance: build-box`, on any node. It fills the node's
//! `cpu.cores`, `bandwidth` and `memory.limit` that the node leaves out, exactly as a table
//! instance does, and what the node writes wins.
//!
//! Names have no `/` (the table's names are `provider/type`), so the two never collide. A custom
//! instance is resolved while the file loads ([`apply`] writes it into the nodes that name it),
//! so everything after sees plain `cpu`, `bandwidth` and `memory` and needs no second code path.

use super::{Config, CpuConfig, Deserialize, JsonSchema, NodeBandwidth, NodeConfig};
use crate::diagnostics::{Located, keys};

/// A machine you describe yourself (an entry of `instances:`).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomInstance {
    /// CPUs, above 0 (at least 0.001) and at most 4096: a number like `16` or `0.5`. Required.
    #[schemars(extend("minimum" = 0.001, "maximum" = 4096))]
    pub cores: f64,
    /// Memory in bytes (`64GB`, `512MiB`); a node that sets a `memory:` takes its `limit` from here
    /// when it leaves it out. Absent = the instance gives no memory limit.
    #[serde(default, deserialize_with = "crate::units::custom_memory")]
    #[schemars(schema_with = "crate::schema::memory_size")]
    pub memory: Option<u64>,
    /// The network card, in bytes per second: one number for both directions or `{ out, in }`, like
    /// a node's `bandwidth`. Absent = the card is not limited.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// What one hour of this machine costs, in US dollars (0 to 1,000,000), for the cost estimate.
    /// Absent = no price: the estimate leaves it out and says so.
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "maximum" = 1_000_000))]
    pub cost_per_hour: Option<f64>,
}

/// Most characters in a custom instance's name.
const MAX_NAME: usize = 64;

/// **Private.** Whether `name` can name a custom instance: 1 to 64 of letters, digits, `.`, `_`
/// and `-`, no `/` (that is the cloud table's `provider/type`).
fn name_ok(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Checks the entries of `instances:` and that every node's `instance:` names one of them or a table
/// type. The error is placed at the entry or at the node's `instance` value.
///
/// # Errors
/// A [`Located`] at the first problem.
pub(super) fn check(cfg: &Config) -> Result<(), Located> {
    for (name, c) in &cfg.instances {
        if !name_ok(name) {
            return Err(Located::at_value(
                keys(&["instances", name.as_str()]),
                format!(
                    "instances: '{name}' is not a usable name (1 to {MAX_NAME} letters, digits, '.', '_' or '-', no '/': that is how the cloud table is written)"
                ),
            ));
        }
        if !(CpuConfig { cores: c.cores }).is_valid() {
            return Err(Located::at_value(
                keys(&["instances", name.as_str(), "cores"]),
                format!(
                    "instance '{name}': cores must be a number from 0.001 to 4096 (got {})",
                    c.cores
                ),
            ));
        }
        if let Some(price) = c.cost_per_hour
            && !(0.0..=1_000_000.0).contains(&price)
        {
            return Err(Located::at_value(
                keys(&["instances", name.as_str(), "cost_per_hour"]),
                format!(
                    "instance '{name}': cost_per_hour must be a number of dollars from 0 to 1000000 (got {price})"
                ),
            ));
        }
        if c.memory == Some(0) {
            return Err(Located::at_value(
                keys(&["instances", name.as_str(), "memory"]),
                format!("instance '{name}': memory must be at least 1 byte"),
            ));
        }
        if c.bandwidth.is_some_and(|b| {
            let (out, rx) = b.pair();
            out == Some(0) || rx == Some(0)
        }) {
            return Err(Located::at_value(
                keys(&["instances", name.as_str(), "bandwidth"]),
                format!("instance '{name}': bandwidth must be at least 1 byte per second"),
            ));
        }
    }
    for (node, n) in &cfg.nodes {
        let Some(i) = n.instance_name() else { continue };
        if !cfg.instances.contains_key(i) && crate::instances::lookup(i).is_none() {
            let mine = if cfg.instances.is_empty() {
                String::new()
            } else {
                format!(
                    "; your instances: {}",
                    cfg.instances.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            };
            return Err(Located::at_value(
                keys(&["nodes", node.as_str(), "instance"]),
                format!(
                    "node '{node}': instance '{i}' is not in the table or in your instances ({}{mine})",
                    crate::instances::hint(i)
                ),
            ));
        }
    }
    Ok(())
}

/// Writes each custom instance into the nodes that name it: `cpu`, `bandwidth` and the `memory`
/// limit the node leaves out. A node that names a table type is left alone (its accessors read the
/// table). Call after [`check`].
pub(super) fn apply(cfg: &mut Config) {
    let custom = cfg.instances.clone();
    for n in cfg.nodes.values_mut() {
        let Some(c) = n.instance_name().and_then(|i| custom.get(i)) else {
            continue;
        };
        n.fill_machine(c);
    }
}

impl NodeConfig {
    /// Fills what this node leaves out from a custom instance: `cpu`, `bandwidth` and the `limit`
    /// of a `memory:` it sets. Does nothing for a kind without them (a cache, an external API, a
    /// storage node).
    fn fill_machine(&mut self, c: &CustomInstance) {
        macro_rules! fill {
            ($n:expr) => {{
                if $n.cpu.is_none() {
                    $n.cpu = Some(CpuConfig { cores: c.cores });
                }
                if $n.bandwidth.is_none() {
                    $n.bandwidth = c.bandwidth;
                }
                if let Some(m) = $n.memory.as_mut()
                    && m.limit.is_none()
                {
                    m.limit = c.memory;
                }
            }};
        }
        match self {
            NodeConfig::Gateway(n) => fill!(n),
            NodeConfig::Service(n) => fill!(n),
            NodeConfig::Monolith(n) => fill!(n),
            NodeConfig::Database(n) => fill!(n),
            NodeConfig::Auth(n) => fill!(n),
            NodeConfig::LoadBalancer(n) => fill!(n),
            NodeConfig::Cache(_) | NodeConfig::ExternalApi(_) | NodeConfig::Storage(_) => {}
        }
    }
}
