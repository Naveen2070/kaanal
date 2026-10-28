//! Named chaos scenarios and the stack that layers them under the live chaos.
//!
//! Pure data and rules: where scenarios come from (config, files, the control API) is the
//! server's business. [`ChaosStack::compile`] folds the active scenarios, bottom to top, and
//! the live state on top, into the [`EffectiveChaos`] the executor reads.
use std::collections::HashSet;

use indexmap::IndexMap;

use crate::chain::{Endpoint, Endpoints, Seq, Step};
use crate::chaos::{ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, compile_layers};
use crate::config::ScenarioConfig;
use crate::diagnostics::{Located, PathSeg};
use crate::pin::{EffectivePins, EmptyOk, PinLayer, PinSource, PinState};
use crate::topology::{NodeId, Topology};

/// Most scenarios a [`ScenarioSet`] holds.
pub const MAX_SCENARIOS: usize = 128;
/// Most entries (keys) one scenario may carry. [`ScenarioSet::insert`] and
/// [`ScenarioSet::replace`] enforce it on the deduplicated layers; a parser should also check
/// the raw entry count before it dedups, so a huge input is refused early.
pub const MAX_ENTRIES: usize = 256;

/// Names the control API (`active`, `save`, `reset`) or Windows file names (devices) trip over.
/// `com1`..`com9` and `lpt1`..`lpt9` are checked separately.
const RESERVED: [&str; 7] = ["active", "save", "reset", "con", "prn", "aux", "nul"];

/// Checks a scenario name: 1..=64 ASCII lowercase letters, digits, `_` or `-`, starting with a
/// letter or digit, and not a reserved word (`active`, `save`, `reset`, Windows device names).
///
/// # Errors
/// Returns a message saying what is wrong with the name.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !ok {
        return Err(format!(
            "invalid scenario name '{name}': use 1-64 chars of a-z, 0-9, '_' or '-', starting with a letter or digit"
        ));
    }
    // The grammar above already guarantees lower case, so no case folding is needed here.
    let device = |p: &str| {
        name.strip_prefix(p)
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
    };
    if RESERVED.contains(&name) || device("com") || device("lpt") {
        return Err(format!("scenario name '{name}' is reserved"));
    }
    Ok(())
}

/// Normalizes `METHOD /path` (method upper-cased), the key `compile` uses for endpoints.
///
/// # Errors
/// Returns a message when there is no method, no path, or the path does not start with `/`.
pub fn endpoint_key(raw: &str) -> Result<String, String> {
    match raw.trim().split_once(' ') {
        Some((m, p)) if !m.is_empty() && p.trim_start().starts_with('/') => {
            Ok(format!("{} {}", m.to_ascii_uppercase(), p.trim_start()))
        }
        // A URL here is a misplaced `--endpoint URL`; it may carry a password, so it is never echoed.
        _ if raw.contains("://") => Err(
            "invalid endpoint: expected 'METHOD /path' (an instance URL goes in the global --endpoint, not a route)"
                .to_string(),
        ),
        _ => Err(format!("invalid endpoint '{raw}': expected 'METHOD /path'")),
    }
}

/// Where a scenario was defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The `scenarios:` section of `kaanal.yaml`.
    Inline,
    /// A scenario file on disk.
    File,
    /// Defined at runtime through the control API.
    Runtime,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Inline => "the config",
            Source::File => "a scenario file",
            Source::Runtime => "the control API",
        }
    }
}

/// Name rules and the entry bound, shared by `insert` and `replace`.
fn check(s: &Scenario) -> Result<(), String> {
    valid_name(&s.name)?;
    let n = s.layers.entry_count() + s.pins.len();
    if n > MAX_ENTRIES {
        return Err(format!(
            "scenario '{}' has {n} entries; at most {MAX_ENTRIES}",
            s.name
        ));
    }
    Ok(())
}

/// A named bundle of chaos layers.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// The name, valid per [`valid_name`].
    pub name: String,
    /// Free text for humans.
    pub description: Option<String>,
    /// The layers this scenario applies when active.
    pub layers: ChaosState,
    /// The pins this scenario applies when active.
    pub pins: PinLayer,
}

/// Every known scenario, in definition order.
#[derive(Debug, Clone, Default)]
pub struct ScenarioSet {
    map: IndexMap<String, (Scenario, Source)>,
}

impl ScenarioSet {
    /// Adds a scenario.
    ///
    /// # Errors
    /// Invalid name, [`MAX_SCENARIOS`] reached, or the name is already defined (the message
    /// names both sources).
    pub fn insert(&mut self, s: Scenario, src: Source) -> Result<(), String> {
        check(&s)?;
        if let Some((_, old)) = self.map.get(&s.name) {
            return Err(if *old == src {
                format!(
                    "scenario '{}' is already defined in {}",
                    s.name,
                    src.label()
                )
            } else {
                format!(
                    "scenario '{}' is defined in both {} and {}",
                    s.name,
                    old.label(),
                    src.label()
                )
            });
        }
        if self.map.len() >= MAX_SCENARIOS {
            return Err(format!("at most {MAX_SCENARIOS} scenarios"));
        }
        self.map.insert(s.name.clone(), (s, src));
        Ok(())
    }

    /// Runtime (re)definition: creates the scenario or overwrites a runtime one.
    ///
    /// # Errors
    /// The name is defined in the config or a file, or [`insert`](Self::insert) would fail.
    pub fn replace(&mut self, s: Scenario) -> Result<(), String> {
        match self.map.get(&s.name) {
            Some((_, Source::Runtime)) => {
                check(&s)?;
                // Same key, so the scenario keeps its position.
                self.map.insert(s.name.clone(), (s, Source::Runtime));
                Ok(())
            }
            Some((_, old)) => Err(format!(
                "scenario '{}' is defined in {}; save a new name or edit the file",
                s.name,
                old.label()
            )),
            None => self.insert(s, Source::Runtime),
        }
    }

    /// Registers a scenario that `save` just wrote to a file: creates it or replaces a file or
    /// runtime one (keeping its position), as [`Source::File`].
    ///
    /// # Errors
    /// The name is defined in the config, or [`insert`](Self::insert) would fail.
    pub fn set_file(&mut self, s: Scenario) -> Result<(), String> {
        match self.map.get(&s.name) {
            Some((_, Source::Inline)) => Err(format!(
                "scenario '{}' is defined in the config; pick another name",
                s.name
            )),
            Some(_) => {
                check(&s)?;
                self.map.insert(s.name.clone(), (s, Source::File));
                Ok(())
            }
            None => self.insert(s, Source::File),
        }
    }

    /// Removes a runtime scenario.
    ///
    /// # Errors
    /// Unknown name, or it is not a runtime scenario.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        match self.map.get(name) {
            None => Err(format!("unknown scenario '{name}'")),
            Some((_, Source::Runtime)) => {
                self.map.shift_remove(name);
                Ok(())
            }
            Some((_, src)) => Err(format!(
                "scenario '{name}' comes from {} and cannot be undefined at runtime",
                src.label()
            )),
        }
    }

    /// A scenario and its source.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<(&Scenario, Source)> {
        self.map.get(name).map(|(s, src)| (s, *src))
    }

    /// Scenario names in definition order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.map.keys().map(String::as_str).collect()
    }
}

/// A problem found while resolving a [`ScenarioConfig`], placed relative to the scenario
/// (`chaos`, entry index, field).
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioError {
    /// Where in the scenario: `[chaos, 2, node]`, `[limits]`, ...
    pub path: Vec<PathSeg>,
    /// Point at the value of the last path entry instead of its key.
    pub value: bool,
    /// What is wrong.
    pub message: String,
    /// The entry names a node or endpoint that does not exist (the control API answers 404 for
    /// this, 400 for everything else).
    pub not_found: bool,
}

impl ScenarioError {
    /// The error as a [`Located`] below `prefix` (`scenarios.<name>` for an inline scenario,
    /// empty for a scenario file).
    #[must_use]
    pub fn located(self, mut prefix: Vec<PathSeg>) -> Located {
        prefix.extend(self.path);
        if self.value {
            Located::at_value(prefix, self.message)
        } else {
            Located::new(prefix, self.message)
        }
    }
}

/// Whether the chain of `ep` visits `node`. Walks the shared bodies once each.
fn visits(ep: &Endpoint, node: NodeId) -> bool {
    if ep.owner == Some(node) || ep.entry_node == Some(node) {
        return true;
    }
    let mut seen: HashSet<*const Seq> = HashSet::new();
    let mut work: Vec<&Seq> = vec![&ep.chain];
    while let Some(seq) = work.pop() {
        if !seen.insert(std::ptr::from_ref(seq)) {
            continue;
        }
        for step in &seq.steps {
            match step {
                Step::Proc { node: n } | Step::Op { node: n, .. } if *n == node => return true,
                Step::Cache(c) if c.node == node => return true,
                Step::Call(c) => {
                    if c.node == node {
                        return true;
                    }
                    work.push(&c.body);
                }
                Step::Parallel(branches) => work.extend(branches.iter()),
                _ => {}
            }
        }
    }
    false
}

/// **Private.** A [`ScenarioError`] from its parts.
fn err(path: Vec<PathSeg>, value: bool, message: String) -> ScenarioError {
    ScenarioError {
        path,
        value,
        message,
        not_found: false,
    }
}

/// **Private.** Like [`err`], for a node or endpoint that is not defined.
fn missing(path: Vec<PathSeg>, message: String) -> ScenarioError {
    ScenarioError {
        not_found: true,
        ..err(path, true, message)
    }
}

mod shape;
pub use shape::validate_scenario_shape;
use shape::{Shaped, ShapedPin, shape};

/// **Private.** Looks one checked entry up in the topology and the endpoints; pushes a warning
/// when the entry's `endpoint` never visits its `node`.
fn resolve_entry(
    name: &str,
    i: usize,
    s: Shaped,
    topo: &Topology,
    endpoints: &Endpoints,
    warnings: &mut Vec<String>,
) -> Result<(ChaosKey, ChaosLayer), ScenarioError> {
    let field = |f: &str| {
        vec![
            PathSeg::Key("chaos".into()),
            PathSeg::Index(i),
            PathSeg::Key(f.into()),
        ]
    };
    if let Some((a, b)) = &s.edge {
        let edge = topo
            .id(a)
            .zip(topo.id(b))
            .and_then(|(x, y)| topo.edge_id(x, y))
            .ok_or_else(|| missing(field("edge"), format!("no edge '{a}' -> '{b}'")))?;
        s.layer
            .check_link(topo.edges[edge].bandwidth)
            .map_err(|(f, m)| err(field(f), true, m))?;
        if s.layer.has_orphan_fault_fields() {
            warnings.push(format!(
                "scenario '{name}': chaos[{i}]: error_kind/error_status have no effect without error_rate"
            ));
        }
        return Ok((ChaosKey::Edge(edge), s.layer));
    }
    let node = match &s.node {
        None => None,
        Some(n) => Some(
            topo.id(n)
                .ok_or_else(|| missing(field("node"), format!("unknown node '{n}'")))?,
        ),
    };
    let endpoint = match s.endpoint {
        None => None,
        Some(key) => {
            let ep = endpoints
                .get(&key)
                .ok_or_else(|| missing(field("endpoint"), format!("unknown endpoint '{key}'")))?;
            if ep.entry_node.is_none() {
                return Err(err(
                    field("endpoint"),
                    true,
                    format!("endpoint '{key}' has no entry node (a static route and no gateway)"),
                ));
            }
            Some((key, ep))
        }
    };
    if let (Some(r), Some(n)) = (s.replica, node) {
        let count = topo.nodes[n].replicas;
        if r >= count {
            return Err(err(
                field("replica"),
                true,
                format!(
                    "node '{}' has {count} replicas (0 to {})",
                    s.node.as_deref().unwrap_or_default(),
                    count - 1
                ),
            ));
        }
    }
    if s.layer.has_orphan_fault_fields() {
        warnings.push(format!(
            "scenario '{name}': chaos[{i}]: error_kind/error_status have no effect without error_rate"
        ));
    }
    let key = match (node, endpoint) {
        (Some(n), None) if s.replica.is_some() => ChaosKey::Replica(n, s.replica.unwrap_or(0)),
        (Some(n), Some((k, ep))) => {
            if !visits(ep, n) {
                warnings.push(format!(
                    "scenario '{name}': chaos[{i}]: endpoint '{k}' never visits node '{}', so this entry never fires",
                    s.node.as_deref().unwrap_or_default()
                ));
            }
            ChaosKey::NodeAt(n, k)
        }
        (Some(n), None) => ChaosKey::Node(n),
        (None, Some((k, _))) => ChaosKey::Endpoint(k),
        (None, None) => unreachable!("shape_entry requires a node or an endpoint"),
    };
    Ok((key, s.layer))
}

/// Turns the parsed form of a scenario into layers, checking it against the topology and the
/// compiled endpoints. Returns the scenario and its warnings (a kind or status without a
/// rate; a `node` + `endpoint` pair the chain never visits). Runs
/// [`validate_scenario_shape`] first.
///
/// # Errors
/// The first problem, placed by entry index and field. Name rules and the scenario count are
/// [`ScenarioSet`]'s business, not checked here.
pub fn resolve_scenario(
    name: &str,
    cfg: &ScenarioConfig,
    topo: &Topology,
    endpoints: &Endpoints,
    empty_ok: EmptyOk,
) -> Result<(Scenario, Vec<String>), ScenarioError> {
    let mut layers = ChaosState::default();
    let mut pins = PinLayer::default();
    let mut warnings = Vec::new();
    let (chaos, shaped_pins) = shape(cfg)?;
    for (i, s) in chaos.into_iter().enumerate() {
        let (key, layer) = resolve_entry(name, i, s, topo, endpoints, &mut warnings)?;
        layers.set(key, layer);
    }
    for (i, p) in shaped_pins.into_iter().enumerate() {
        resolve_pin(i, &p, endpoints, empty_ok)?;
        pins.insert(p.key, p.entry);
    }
    Ok((
        Scenario {
            name: name.to_string(),
            description: cfg.description.clone(),
            layers,
            pins,
        },
        warnings,
    ))
}

/// **Private.** Looks one checked pin up in the endpoints: it must exist, be a real route (not
/// a synthesized `HEAD`), enter the chain, and for `empty` have an empty form.
fn resolve_pin(
    i: usize,
    p: &ShapedPin,
    endpoints: &Endpoints,
    empty_ok: EmptyOk,
) -> Result<(), ScenarioError> {
    let field = |f: &str| {
        vec![
            PathSeg::Key("pins".into()),
            PathSeg::Index(i),
            PathSeg::Key(f.into()),
        ]
    };
    let key = &p.key;
    let ep = endpoints
        .get(key)
        .ok_or_else(|| missing(field("endpoint"), format!("unknown endpoint '{key}'")))?;
    if ep.synthesized {
        return Err(err(
            field("endpoint"),
            true,
            format!("'{key}' is a HEAD made from its GET; pin the GET route"),
        ));
    }
    if ep.entry_node.is_none() {
        return Err(err(
            field("endpoint"),
            true,
            format!("endpoint '{key}' has no entry node (a static route and no gateway)"),
        ));
    }
    if p.entry.state == PinState::Empty && !empty_ok(ep) {
        return Err(err(
            field("state"),
            true,
            format!("no empty form for '{key}'; add states.empty.body to its endpoints entry"),
        ));
    }
    Ok(())
}

/// The active scenarios (an ordered stack) plus the live chaos that always sits on top.
#[derive(Debug, Clone, Default)]
pub struct ChaosStack {
    active: Vec<String>,
    /// Ad-hoc chaos set through the control API; wins over every scenario.
    live: ChaosState,
    /// Ad-hoc pins; win over every scenario's.
    live_pins: PinLayer,
}

impl ChaosStack {
    /// The live chaos, which sits on top of every scenario.
    #[must_use]
    pub fn live(&self) -> &ChaosState {
        &self.live
    }

    /// The live chaos, to change it.
    pub fn live_mut(&mut self) -> &mut ChaosState {
        &mut self.live
    }

    /// The live pins, which sit on top of every scenario's.
    #[must_use]
    pub fn live_pins(&self) -> &PinLayer {
        &self.live_pins
    }

    /// The live pins, to change them.
    pub fn live_pins_mut(&mut self) -> &mut PinLayer {
        &mut self.live_pins
    }

    /// Activates a scenario on top of the stack (moving it there if already active).
    ///
    /// # Errors
    /// Unknown scenario.
    pub fn activate(&mut self, set: &ScenarioSet, name: &str) -> Result<(), String> {
        if set.get(name).is_none() {
            return Err(format!("unknown scenario '{name}'"));
        }
        self.active.retain(|n| n != name);
        self.active.push(name.to_string());
        Ok(())
    }

    /// Removes a scenario from the stack; false when it was not active.
    pub fn deactivate(&mut self, name: &str) -> bool {
        let n = self.active.len();
        self.active.retain(|a| a != name);
        self.active.len() != n
    }

    /// Replaces the whole stack (bottom to top). Atomic: any unknown name changes nothing.
    ///
    /// # Errors
    /// Unknown scenario.
    pub fn set_active(&mut self, set: &ScenarioSet, names: &[String]) -> Result<(), String> {
        if let Some(bad) = names.iter().find(|n| set.get(n).is_none()) {
            return Err(format!("unknown scenario '{bad}'"));
        }
        let mut next: Vec<String> = Vec::with_capacity(names.len());
        for n in names {
            next.retain(|a| a != n);
            next.push(n.clone());
        }
        self.active = next;
        Ok(())
    }

    /// Drops active names that are no longer in `set`. The server's undefine (`DELETE`) handler
    /// must call this (or [`deactivate`](Self::deactivate)) after [`ScenarioSet::remove`], so
    /// `active()` never reports a scenario that does not exist.
    pub fn prune(&mut self, set: &ScenarioSet) {
        self.active.retain(|n| set.get(n).is_some());
    }

    /// Deactivates every scenario; live chaos stays.
    pub fn clear_scenarios(&mut self) {
        self.active.clear();
    }

    /// Deactivates every scenario and clears the live chaos and the live pins.
    pub fn clear_all(&mut self) {
        self.active.clear();
        self.live.clear();
        self.live_pins.clear();
    }

    /// Active scenario names, bottom to top.
    #[must_use]
    pub fn active(&self) -> &[String] {
        &self.active
    }

    /// Folds the active scenarios (bottom to top, skipping any no longer defined) and the live
    /// state (last, so it wins) into what the executor reads.
    #[must_use]
    pub fn compile(&self, set: &ScenarioSet) -> EffectiveChaos {
        let mut layers: Vec<&ChaosState> = self
            .active
            .iter()
            .filter_map(|n| set.get(n).map(|(s, _)| &s.layers))
            .collect();
        layers.push(&self.live);
        compile_layers(&layers)
    }

    /// Folds the pins of the active scenarios (bottom to top) and the live pins (last, so
    /// they win). A later layer replaces an endpoint's pin whole; a `success` pin is kept so
    /// it masks a lower one.
    #[must_use]
    pub fn compile_pins(&self, set: &ScenarioSet) -> EffectivePins {
        let mut out = EffectivePins::default();
        for n in &self.active {
            if let Some((s, _)) = set.get(n) {
                out.overlay(&s.pins, &PinSource::Scenario(n.clone()));
            }
        }
        out.overlay(&self.live_pins, &PinSource::Live);
        out
    }
}
