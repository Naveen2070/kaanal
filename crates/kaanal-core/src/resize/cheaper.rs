//! Cheaper-move candidates (Step 9c-2, plan Task 2): every single-knob move of a node to a strictly
//! cheaper value, for the cheapest search to shrink a design. Pure: no simulation, no I/O.
//!
//! - A table class: the **Pareto front** over (vCPU up, memory up, price down) of the classes of the
//!   node's provider and sort (compute, managed database or managed cache; the sort's own price list,
//!   [`crate::cost::class_price`]) that cost less than the current one, of any series. A class is
//!   dropped when another candidate has at least its vCPU and memory for no more money (the
//!   maximal-vector / skyline problem: Kung, Luccio and Preparata, "On finding the maxima of a set
//!   of vectors", JACM 22(4), 1975; Borzsonyi et al., "The Skyline Operator", ICDE 2001). Twins with
//!   the same vCPU, memory and price keep the first name. Sorted by (price, vCPU down, memory down,
//!   name), every dominator precedes what it dominates, so one sort and one sweep against the kept
//!   front decide it: O(n log n + n f) for n classes and a front of f (f is a handful).
//! - Copies from 1 up to one below the current count, where copies may change at all.
//!
//! The list is ordered ascending by the price of the resulting node (class price times copies), so a
//! search can bisect it. Empty for a node in `keep`, a node with no instance (it has no price, and
//! the cheapest search never shrinks what costs nothing, RULES 142 e), a custom instance, a class
//! with no price, or a kind with none of these. As in [`super::moves_for`], a class is not offered where the developer
//! wrote `cpu` or `memory.limit` (the class would not change it); copies still are.

use crate::config::{Config, NodeConfig};
use crate::instances::{self, Instance};

use super::moves::{Move, MoveKind, copies_blocked, target};

/// Every single-knob move of `node` to a strictly cheaper value (see the module doc), cheapest
/// resulting node first.
#[must_use]
pub fn cheaper_moves(cfg: &Config, node: &str, keep: &[String]) -> Vec<Move> {
    let Ok(n) = target(cfg, node, keep) else {
        return Vec::new();
    };
    let copies = u64::from(n.replicas());
    // (price of the resulting node, order among equals, the move)
    let mut v: Vec<(u64, u8, MoveKind)> = Vec::new();
    let Some(name) = n.instance_name() else {
        return Vec::new();
    };
    if cfg.instances.contains_key(name) {
        return Vec::new();
    }
    let Some((cur, unit)) = priced(cfg, n, name) else {
        return Vec::new();
    };
    if n.own_cpu().is_none() && n.memory_config().and_then(|m| m.limit).is_none() {
        for (i, q) in front(cfg, n, cur, unit) {
            let kind = MoveKind::Class {
                from: cur.name.to_string(),
                to: i.name.to_string(),
            };
            v.push((q * copies, 0, kind));
        }
    }
    if copies_blocked(cfg, node, n).is_none() {
        for to in 1..n.replicas() {
            let kind = MoveKind::Copies {
                from: n.replicas(),
                to,
            };
            v.push((unit * u64::from(to), 1, kind));
        }
    }
    // stable: equal keys keep the order they were pushed in (smallest first)
    v.sort_by_key(|(p, o, _)| (*p, *o));
    v.into_iter()
        .map(|(_, _, kind)| Move {
            node: node.to_string(),
            kind,
        })
        .collect()
}

/// **Private.** The table class `name` and its price an hour in the region the file prices its
/// cloud in, or `None` when it has no price there.
fn priced(cfg: &Config, n: &NodeConfig, name: &str) -> Option<(&'static Instance, u64)> {
    let cur = instances::lookup(name)?;
    let provider = cur.name.split_once('/').map_or("", |(p, _)| p);
    let region = crate::cost::regions_of(cfg);
    let p = crate::cost::class_price(n, provider, region.get(provider)?, cur.name)?;
    Some((cur, u64::from(p)))
}

/// **Private.** The Pareto front of the classes cheaper than `cur` (priced `p`): same provider and
/// sort, any series, with their prices, cheapest first.
fn front(cfg: &Config, n: &NodeConfig, cur: &Instance, p: u64) -> Vec<(&'static Instance, u64)> {
    let provider = cur.name.split_once('/').map_or("", |(p, _)| p);
    let regions = crate::cost::regions_of(cfg);
    let Some(region) = regions.get(provider) else {
        return Vec::new();
    };
    let sort = |c: &str| (instances::is_db_class(c), instances::is_cache_class(c));
    let mut v: Vec<(&'static Instance, u64)> = instances::all()
        .iter()
        .filter(|i| {
            i.name
                .strip_prefix(provider)
                .is_some_and(|r| r.starts_with('/'))
                && sort(i.name) == sort(cur.name)
        })
        .filter_map(|i| {
            let q = u64::from(crate::cost::class_price(n, provider, region, i.name)?);
            (q < p).then_some((i, q))
        })
        .collect();
    v.sort_by_key(|(i, q)| {
        (
            *q,
            std::cmp::Reverse(i.vcpu),
            std::cmp::Reverse(i.memory_mib),
            i.name,
        )
    });
    let mut kept: Vec<(&'static Instance, u64)> = Vec::new();
    for (i, q) in v {
        if !kept
            .iter()
            .any(|(k, _)| k.vcpu >= i.vcpu && k.memory_mib >= i.memory_mib)
        {
            kept.push((i, q));
        }
    }
    kept
}
