//! Keeping what still applies across an Apply. Chaos, scenarios, leases and pins name things by
//! topology id (a node index, an edge index) or by endpoint key; a new draft renumbers the ids
//! and may remove the things. Everything is carried over **by name**, and what names something
//! the new system lacks, or something the new system would not accept at all, is dropped and
//! reported. A rename is a removal plus an addition.

use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosStack, ChaosState, EdgeId, KeyMap, NodeId, PinEntry, PinLayer,
    PinState, Scenario, ScenarioSet, Source, Topology,
};
use serde_json::{Value, json};

use crate::chaos_lease;
use crate::control::key_label;
use crate::generation::Generation;
use crate::state::Control;

/// What a remap kept and dropped, in the shapes the Apply answer carries.
pub(crate) struct Report {
    /// `{ live_chaos: n, active_scenarios: [names], runtime_scenarios: [names], pins: n }`.
    pub(crate) kept: Value,
    /// `{ live_chaos: [labels], scenario_chaos: [labels], active_scenarios: [names], pins: [keys] }`.
    pub(crate) dropped: Value,
}

impl Default for Report {
    fn default() -> Report {
        Report {
            kept: json!({
                "live_chaos": 0, "active_scenarios": [], "runtime_scenarios": [], "pins": 0
            }),
            dropped: json!({
                "live_chaos": [], "scenario_chaos": [], "active_scenarios": [], "pins": []
            }),
        }
    }
}

/// The old topology's ids as the new generation knows them, by name.
struct ByName<'a> {
    old: &'a Topology,
    new: &'a Generation,
}

impl KeyMap for ByName<'_> {
    fn node(&self, old: NodeId) -> Option<NodeId> {
        self.new.topo.id(&self.old.nodes.get(old)?.name)
    }

    fn edge(&self, old: EdgeId) -> Option<EdgeId> {
        let e = self.old.edges.get(old)?;
        self.new.topo.edge_id(self.node(e.from)?, self.node(e.to)?)
    }

    fn replica_exists(&self, new_node: NodeId, replica: u16) -> bool {
        self.new
            .topo
            .nodes
            .get(new_node)
            .is_some_and(|n| replica < n.replicas)
    }

    /// The control API takes chaos on an endpoint only when it has an entry node.
    fn endpoint_exists(&self, endpoint: &str) -> bool {
        self.new
            .endpoints
            .get(endpoint)
            .is_some_and(|ep| ep.entry_node.is_some())
    }
}

/// Whether the control API would accept a pin on `key` in `new`: the endpoint exists, is not a
/// `HEAD` made from its `GET`, has an entry node, and (for `empty`) has an empty form.
fn pin_ok(new: &Generation, key: &str, entry: &PinEntry) -> bool {
    new.endpoints.get(key).is_some_and(|ep| {
        !ep.synthesized
            && ep.entry_node.is_some()
            && (entry.state != PinState::Empty || new.empty_ok_for(ep))
    })
}

/// A layer on an edge may only cap bandwidth the edge has, and only below it. Where the new
/// topology no longer allows the cap, the cap is removed (the rest of the layer stays, and an
/// emptied layer goes); returns the keys that lost one, labelled by the new topology.
fn drop_stale_caps(state: &mut ChaosState, new: &Generation) -> Vec<String> {
    let stale: Vec<ChaosKey> = state
        .entries()
        .into_iter()
        .filter_map(|(k, l)| {
            let (ChaosKey::Edge(e), Some(cap)) = (&k, l.bandwidth_cap) else {
                return None;
            };
            let allowed = new.topo.edges[*e].bandwidth.is_some_and(|bw| cap <= bw);
            (!allowed).then_some(k)
        })
        .collect();
    let mut labels = Vec::new();
    for k in stale {
        let mut layer: ChaosLayer = state.layer(&k).cloned().unwrap_or_default();
        layer.bandwidth_cap = None;
        labels.push(format!("{} bandwidth_cap", key_label(new, &k)));
        if layer == ChaosLayer::default() {
            state.remove(&k);
        } else {
            state.set(k, layer);
        }
    }
    labels
}

/// Carries the runtime scenarios (defined through the API, so in no file) into `scenarios`: by
/// name, with their chaos remapped and their pins checked like the live ones. A runtime scenario
/// whose name the new draft now defines is replaced by the draft's definition.
fn carry_runtime(
    old: &Generation,
    new: &Generation,
    map: &ByName,
    from: &ScenarioSet,
    scenarios: &mut ScenarioSet,
    dropped: &mut Vec<String>,
) -> Vec<String> {
    let mut kept = Vec::new();
    for name in from.names() {
        let Some((s, Source::Runtime)) = from.get(name) else {
            continue;
        };
        if scenarios.get(name).is_some() {
            dropped.push(format!(
                "scenario {name}: replaced by the draft's definition"
            ));
            continue;
        }
        let (mut layers, gone) = s.layers.remap(map);
        dropped.extend(
            gone.iter()
                .map(|k| format!("scenario {name}: {}", key_label(old, k))),
        );
        dropped.extend(
            drop_stale_caps(&mut layers, new)
                .into_iter()
                .map(|l| format!("scenario {name}: {l}")),
        );
        let mut pins = PinLayer::default();
        for (key, entry) in s.pins.iter() {
            if pin_ok(new, key, entry) {
                pins.insert(key.clone(), *entry);
            } else {
                dropped.push(format!("scenario {name}: pin {key}"));
            }
        }
        let carried = Scenario {
            name: s.name.clone(),
            description: s.description.clone(),
            layers,
            pins,
        };
        match scenarios.insert(carried, Source::Runtime) {
            Ok(()) => kept.push(name.to_string()),
            Err(m) => dropped.push(format!("scenario {name}: {m}")),
        }
    }
    kept
}

/// Rebuilds `c` for the generation `new`, from what it held for `old`: the live chaos and the
/// chaos leases by name, the runtime scenarios, the active scenarios that the new set still
/// defines, and the live pins the control API would still accept. The caller holds the `control`
/// lock and publishes afterwards.
///
/// # Errors
/// The active stack could not be rebuilt; `c` is left as it was.
pub(crate) fn remap_control(
    old: &Generation,
    new: &Generation,
    mut scenarios: ScenarioSet,
    c: &mut Control,
) -> Result<Report, String> {
    let map = ByName {
        old: &old.topo,
        new,
    };
    let mut dropped_scenario_chaos = Vec::new();
    let runtime = carry_runtime(
        old,
        new,
        &map,
        &c.scenarios,
        &mut scenarios,
        &mut dropped_scenario_chaos,
    );

    let (mut live, dropped_keys) = c.stack.live().remap(&map);
    let mut dropped_live: Vec<String> = dropped_keys.iter().map(|k| key_label(old, k)).collect();
    dropped_live.extend(drop_stale_caps(&mut live, new));

    let mut pins = PinLayer::default();
    let mut dropped_pins = Vec::new();
    for (key, entry) in c.stack.live_pins().iter() {
        if pin_ok(new, key, entry) {
            pins.insert(key.clone(), *entry);
        } else {
            dropped_pins.push(key.clone());
        }
    }

    let (kept_active, dropped_active): (Vec<String>, Vec<String>) = c
        .stack
        .active()
        .iter()
        .cloned()
        .partition(|n| scenarios.get(n).is_some());
    let mut stack = ChaosStack::default();
    *stack.live_mut() = live;
    *stack.live_pins_mut() = pins;
    stack
        .set_active(&scenarios, &kept_active)
        .map_err(|m| format!("rebuilding the active scenarios: {m}"))?;

    let mut chaos_leases: chaos_lease::Leases = c
        .chaos_leases
        .iter()
        .filter_map(|((k, u), d)| Some(((k.remap(&map)?, *u), *d)))
        .collect();
    chaos_lease::prune(&mut chaos_leases, stack.live());
    let leases = c
        .leases
        .iter()
        .filter(|(n, _)| kept_active.contains(n))
        .map(|(n, d)| (n.clone(), *d))
        .collect();

    let report = Report {
        kept: json!({
            "live_chaos": stack.live().entry_count(),
            "active_scenarios": kept_active,
            "runtime_scenarios": runtime,
            "pins": stack.live_pins().len(),
        }),
        dropped: json!({
            "live_chaos": dropped_live,
            "scenario_chaos": dropped_scenario_chaos,
            "active_scenarios": dropped_active,
            "pins": dropped_pins,
        }),
    };
    *c = Control {
        scenarios,
        stack,
        leases,
        chaos_leases,
    };
    Ok(report)
}
