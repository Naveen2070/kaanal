//! The move rules (RULES 141 (b), plan Task 2): one step of each size knob a node offers.
//!
//! - A node with a table instance steps up its class (the next rung of [`instances::ladder_by`]: the
//!   same series, priced in the region the file prices that cloud in) and adds a copy; never cores.
//! - A node with no instance steps its written `cpu` to the next of 1, 2, 4 ... 64 and adds a copy.
//!   With no `cpu` written its cores are not limited, so a core count would only cap it: copies only.
//! - A cache adds copies only (its cores are not modelled, RULES 122).
//! - A figure the developer wrote that a class move would leave in force or discard (an own `cpu`,
//!   an own `memory.limit` next to a table instance) or a custom instance from `instances:` (its
//!   figures are written into the node at load, so own and filled cannot be told apart; RULES 141 (c)
//!   lets it start pinned) is never overridden: copies only, and [`copies_only_reason`] says why.
//! - An external API and an object store have no machine and no copies to buy: no moves.
//! - Copies stop at [`MAX_REPLICAS`] and are refused where a `weighted` balance (on the node or on
//!   an edge into it) names each copy.
//!
//! Memory moves only on request ([`moves_for_oom`], when the cause is `oom`): the next class up on a
//! node with a table instance, else the written limit doubled.

use serde::Serialize;

use crate::balance::{Balance, MAX_REPLICAS};
use crate::config::{Config, CpuConfig, MemoryConfig, NodeConfig, NodeKind};
use crate::instances;

/// One step of one knob, as `from` and `to`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "knob", rename_all = "lowercase")]
pub enum MoveKind {
    /// The `instance:` to the next class up.
    Class {
        /// The class the node has.
        from: String,
        /// The next class up its ladder.
        to: String,
    },
    /// The written `cpu.cores` to the next step.
    Cores {
        /// Cores now.
        from: f64,
        /// Cores after.
        to: f64,
    },
    /// The written `memory.limit`, in bytes, doubled.
    Memory {
        /// Bytes now.
        from: u64,
        /// Bytes after.
        to: u64,
    },
    /// `replicas` plus one.
    Copies {
        /// Copies now.
        from: u16,
        /// Copies after.
        to: u16,
    },
}

/// A move on one node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Move {
    /// The node's name.
    pub node: String,
    /// What changes.
    pub kind: MoveKind,
}

/// **Private.** The core counts a `Cores` move steps along, in millicores.
const CORE_STEPS: [u32; 7] = [1000, 2000, 4000, 8000, 16_000, 32_000, 64_000];

/// The moves `node` offers for load: its size knob (class or cores) and one more copy, each when
/// the rules allow. Empty for a node in `keep`, an unknown node, an external API or an object store.
#[must_use]
pub fn moves_for(cfg: &Config, node: &str, keep: &[String]) -> Vec<Move> {
    moves(cfg, node, keep, false)
}

/// [`moves_for`] when the cause is `oom`: memory instead of cores (the next class up on a node with
/// a table instance, else the written limit doubled), and one more copy.
#[must_use]
pub fn moves_for_oom(cfg: &Config, node: &str, keep: &[String]) -> Vec<Move> {
    moves(cfg, node, keep, true)
}

/// The plain reason `node` has no move at all (`"kept by you"`, `"an external service"`, ...);
/// `None` when [`moves_for`] gives it at least one. This follows the load rules: for an `oom`
/// cause, ask whether [`moves_for_oom`] is empty (a written memory limit gives a move here that
/// load does not have, and the reverse).
#[must_use]
pub fn why_fixed(cfg: &Config, node: &str, keep: &[String]) -> Option<&'static str> {
    let n = match target(cfg, node, keep) {
        Ok(n) => n,
        Err(why) => return Some(why),
    };
    match (size_move(cfg, n, false), copies(cfg, node, n)) {
        // the copy is the knob a developer can unblock, so its reason is the one given
        (Err(_), Err(why)) => Some(why),
        _ => None,
    }
}

/// Why `node` moves by copies only although it has a machine (`"your own cpu is kept ..."`); `None`
/// when it has a size move, no copy move, or no moves at all ([`why_fixed`] covers that).
#[must_use]
pub fn copies_only_reason(cfg: &Config, node: &str, keep: &[String]) -> Option<&'static str> {
    let n = target(cfg, node, keep).ok()?;
    match (size_move(cfg, n, false), copies(cfg, node, n)) {
        (Err(why), Ok(_)) => Some(why),
        _ => None,
    }
}

/// Applies `mv` to `cfg` (a clone of the design: `instance`, `cpu.cores`, `memory.limit` or
/// `replicas`), then [`Config::validate_candidate`].
///
/// # Errors
/// The node is unknown, the move's `from` is not what the node has (a stale move), the field does
/// not exist on its kind, the class is not in the table, or the check fails; `cfg` is then as it was.
pub fn apply(cfg: &mut Config, mv: &Move) -> Result<(), String> {
    let at = |e: &str| format!("node '{}': {e}", mv.node);
    let Some(n) = cfg.nodes.get_mut(&mv.node) else {
        return Err(at("not in the design"));
    };
    let before = n.clone();
    if let Err(e) = set(n, &mv.kind) {
        *n = before;
        return Err(at(e));
    }
    if let Err(e) = cfg.validate_candidate() {
        cfg.nodes[&mv.node] = before;
        return Err(e);
    }
    Ok(())
}

/// **Private.** The node the rules look at, or why there is none to look at.
fn target<'a>(
    cfg: &'a Config,
    node: &str,
    keep: &[String],
) -> Result<&'a NodeConfig, &'static str> {
    if keep.iter().any(|k| k == node) {
        return Err("kept by you");
    }
    let n = cfg.nodes.get(node).ok_or("not in the design")?;
    match n.kind() {
        NodeKind::ExternalApi => Err("an external service"),
        NodeKind::Storage => Err("an object store"),
        _ => Ok(n),
    }
}

/// **Private.** The moves of [`moves_for`] (`oom` false) or [`moves_for_oom`].
fn moves(cfg: &Config, node: &str, keep: &[String], oom: bool) -> Vec<Move> {
    let Ok(n) = target(cfg, node, keep) else {
        return Vec::new();
    };
    [size_move(cfg, n, oom), copies(cfg, node, n)]
        .into_iter()
        .flatten()
        .map(|kind| Move {
            node: node.to_string(),
            kind,
        })
        .collect()
}

/// **Private.** The node's size knob (class, cores, or memory when `oom`), or why it has none.
fn size_move(cfg: &Config, n: &NodeConfig, oom: bool) -> Result<MoveKind, &'static str> {
    if n.kind() == NodeKind::Cache {
        return Err("a cache: its cores are not modelled, so it grows by copies");
    }
    if let Some(name) = n.instance_name() {
        if cfg.instances.contains_key(name) {
            return Err("your own instance: its figures are yours, so it grows by copies");
        }
        if n.own_cpu().is_some() {
            return Err("your own cpu is kept: a larger class would not change it");
        }
        if n.memory_config().and_then(|m| m.limit).is_some() {
            return Err("your own memory limit is kept: a larger class would not change it");
        }
        return next_class(cfg, n, name, oom).map(|to| MoveKind::Class {
            from: name.to_string(),
            to: to.to_string(),
        });
    }
    if oom {
        let from = n
            .memory_config()
            .and_then(|m| m.limit)
            .ok_or("no memory limit written")?;
        let to = from
            .checked_mul(2)
            .ok_or("the memory limit cannot double")?;
        return Ok(MoveKind::Memory { from, to });
    }
    let c = n
        .own_cpu()
        .ok_or("no cpu written: its cores are not limited, so it grows by copies")?;
    let to = CORE_STEPS
        .iter()
        .find(|s| **s > c.millicores())
        .ok_or("at the most cores the planner steps to (64)")?;
    Ok(MoveKind::Cores {
        from: c.cores,
        to: f64::from(*to) / 1000.0,
    })
}

/// **Private.** The next class up the ladder of the table class `name`, in the region the file
/// prices its cloud in.
///
/// The rungs are priced from the list the node's hosting and engine use (a managed database from its
/// engine's list, not the compute one). A load step goes to the first rung with more vCPU and no
/// less memory, an `oom` step to the first with more memory and no fewer vCPU, so a step never
/// lowers the knob it does not move, never swaps a family for one with the same size, and never
/// lands on a twin (`m5.24xlarge` to `m5.metal`). Rungs come cheapest first within a vCPU count.
fn next_class(
    cfg: &Config,
    n: &NodeConfig,
    name: &str,
    oom: bool,
) -> Result<&'static str, &'static str> {
    let cur = instances::lookup(name).ok_or("its class is not in the table")?;
    if instances::series(cur.name).is_none() {
        return Err("its class name shows no size, so no larger class is known");
    }
    let provider = cur.name.split_once('/').map_or("", |(p, _)| p);
    let regions = crate::cost::regions_of(cfg);
    let region = regions.get(provider).ok_or("its cloud has no price list")?;
    let ladder = instances::ladder_by(cur.name, |c| {
        crate::cost::class_price(n, provider, region, c)
    });
    if ladder.is_empty() {
        return Err("its class has no price in this region for this engine");
    }
    ladder
        .iter()
        .find(|i| {
            if oom {
                i.memory_mib > cur.memory_mib && i.vcpu >= cur.vcpu
            } else {
                i.vcpu > cur.vcpu && i.memory_mib >= cur.memory_mib
            }
        })
        .map(|i| i.name)
        .ok_or("the largest class of its series in the region's price list")
}

/// **Private.** One more copy, or why not.
fn copies(cfg: &Config, node: &str, n: &NodeConfig) -> Result<MoveKind, &'static str> {
    let from = n.replicas();
    if from >= MAX_REPLICAS {
        return Err("at the most copies (64)");
    }
    // each copy has its own limiter, so a copy would raise the limit the user set (spec: a plan
    // never raises a user-set rate limit)
    if n.rate_limit().is_some() {
        return Err("it has a rate limit you set, and each copy would add its own");
    }
    let weighted = |b: Option<&Balance>| matches!(b, Some(Balance::Weighted(_)));
    if weighted(n.balance())
        || cfg
            .edges
            .iter()
            .any(|e| e.to == node && weighted(e.balance.as_ref()))
    {
        return Err("a weighted balance names each copy: one more needs a weight from you");
    }
    Ok(MoveKind::Copies { from, to: from + 1 })
}

/// **Private.** Writes `kind` into `n` after checking its `from` is what `n` has.
fn set(n: &mut NodeConfig, kind: &MoveKind) -> Result<(), &'static str> {
    const STALE: &str = "the move is stale: the node no longer has its 'from'";
    const NO_FIELD: &str = "its kind has no such field";
    match kind {
        MoveKind::Class { from, to } => {
            if instances::lookup(to).is_none() {
                return Err("the class is not in the table");
            }
            let slot = instance_slot(n).ok_or(NO_FIELD)?;
            if slot.as_deref() != Some(from.as_str()) {
                return Err(STALE);
            }
            *slot = Some(to.clone());
        }
        MoveKind::Cores { from, to } => {
            let slot = cpu_slot(n).ok_or(NO_FIELD)?;
            if slot.map(CpuConfig::millicores) != Some(CpuConfig { cores: *from }.millicores()) {
                return Err(STALE);
            }
            *slot = Some(CpuConfig { cores: *to });
        }
        MoveKind::Memory { from, to } => {
            let m = memory_slot(n).ok_or(NO_FIELD)?.as_mut().ok_or(STALE)?;
            if m.limit != Some(*from) {
                return Err(STALE);
            }
            m.limit = Some(*to);
        }
        MoveKind::Copies { from, to } => {
            if n.replicas() != *from {
                return Err(STALE);
            }
            *replicas_slot(n) = Some(*to);
        }
    }
    Ok(())
}

/// **Private.** `fn $name(&mut NodeConfig) -> Option<&mut $t>`: the field on the kinds that have it.
macro_rules! slot {
    ($name:ident, $field:ident, $t:ty, [$($k:ident),*]) => {
        fn $name(n: &mut NodeConfig) -> Option<&mut $t> {
            match n {
                $(NodeConfig::$k(c) => Some(&mut c.$field),)*
                _ => None,
            }
        }
    };
}

slot!(
    cpu_slot,
    cpu,
    Option<CpuConfig>,
    [Gateway, Service, Monolith, Database, Auth, LoadBalancer]
);
slot!(
    memory_slot,
    memory,
    Option<MemoryConfig>,
    [Gateway, Service, Monolith, Database, Auth, LoadBalancer]
);
slot!(
    instance_slot,
    instance,
    Option<String>,
    [
        Gateway,
        Service,
        Monolith,
        Database,
        Auth,
        LoadBalancer,
        Cache
    ]
);

/// **Private.** The `replicas:` field (every kind has one).
fn replicas_slot(n: &mut NodeConfig) -> &mut Option<u16> {
    match n {
        NodeConfig::Gateway(c) => &mut c.replicas,
        NodeConfig::Service(c) => &mut c.replicas,
        NodeConfig::Monolith(c) => &mut c.replicas,
        NodeConfig::Database(c) => &mut c.replicas,
        NodeConfig::Cache(c) => &mut c.replicas,
        NodeConfig::ExternalApi(c) => &mut c.replicas,
        NodeConfig::Auth(c) => &mut c.replicas,
        NodeConfig::LoadBalancer(c) => &mut c.replicas,
        NodeConfig::Storage(c) => &mut c.replicas,
    }
}
