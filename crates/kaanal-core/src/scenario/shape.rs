//! The topology-free checks of a scenario file: what [`validate_scenario_shape`] runs and
//! [`super::resolve_scenario`] runs first. Moved out of `mod.rs` (R3.8).

use std::collections::HashMap;
use std::time::Duration;

use super::{MAX_ENTRIES, ScenarioError, endpoint_key, err};
use crate::chaos::{ChaosLayer, ErrorKind, MAX_LATENCY_ADD};
use crate::config::{ChaosEntry, ScenarioConfig};
use crate::diagnostics::PathSeg;
use crate::pin::{PinEntry, check_entry};

/// **Private.** What makes two chaos entries of one scenario the same key.
type EntryKey = (
    Option<String>,
    Option<String>,
    Option<u16>,
    Option<(String, String)>,
);

/// **Private.** One entry after the topology-free checks: where it points (the node as
/// written, the endpoint normalized) and the layer it sets.
pub(super) struct Shaped {
    pub(super) node: Option<String>,
    pub(super) endpoint: Option<String>,
    pub(super) replica: Option<u16>,
    pub(super) edge: Option<(String, String)>,
    pub(super) layer: ChaosLayer,
}

/// **Private.** The checks of one entry that need no topology: something to point at, a
/// well-formed endpoint, a known kind, something set, every value in range.
fn shape_entry(i: usize, e: &ChaosEntry) -> Result<Shaped, ScenarioError> {
    let item = || vec![PathSeg::Key("chaos".into()), PathSeg::Index(i)];
    let field = |f: &str| {
        let mut p = item();
        p.push(PathSeg::Key(f.into()));
        p
    };
    if e.edge.is_some() && (e.node.is_some() || e.endpoint.is_some() || e.replica.is_some()) {
        return Err(err(
            field("edge"),
            true,
            "an edge entry takes no node, endpoint or replica".into(),
        ));
    }
    if e.node.is_none() && e.endpoint.is_none() && e.edge.is_none() {
        return Err(err(
            item(),
            false,
            "entry needs a node, endpoint or edge".into(),
        ));
    }
    if e.replica.is_some() && e.node.is_none() {
        return Err(err(field("replica"), true, "replica needs a node".into()));
    }
    if e.replica.is_some() && e.endpoint.is_some() {
        return Err(err(
            field("replica"),
            true,
            "a replica entry takes no endpoint (it applies whatever the endpoint)".into(),
        ));
    }
    let endpoint = match &e.endpoint {
        None => None,
        Some(raw) => Some(endpoint_key(raw).map_err(|m| err(field("endpoint"), true, m))?),
    };
    let kind = match &e.error_kind {
        None => None,
        Some(k) => Some(ErrorKind::parse(k).ok_or_else(|| {
            err(
                field("error_kind"),
                true,
                format!("error_kind '{k}' must be one of status, reset, hang, malformed"),
            )
        })?),
    };
    let layer = ChaosLayer {
        latency_add: e.latency_ms.map(Duration::from_millis),
        error_rate: e.error_rate,
        error_kind: kind,
        error_status: e.error_status,
        down: e.down,
        jitter: e.jitter_ms.map(Duration::from_millis),
        bandwidth_cap: e.bandwidth_cap,
        replace: None,
    };
    if layer == ChaosLayer::default() {
        return Err(err(
            item(),
            false,
            "entry sets nothing (add latency_ms, error_rate, error_kind, error_status, down, jitter_ms or bandwidth_cap)"
                .into(),
        ));
    }
    if let Err((f, m)) = layer.check() {
        return Err(if f == "latency_add" {
            err(
                field("latency_ms"),
                true,
                format!(
                    "latency_ms must be at most {} ms (5 min)",
                    MAX_LATENCY_ADD.as_millis()
                ),
            )
        } else {
            err(field(f), true, m)
        });
    }
    // An edge's own rules; the cap against its real bandwidth waits for the topology.
    if e.edge.is_some() {
        layer
            .check_link(Some(u64::MAX))
            .map_err(|(f, m)| err(field(f), true, m))?;
    } else if let Some(f) = layer.link_only_field() {
        let name = if f == "jitter" { "jitter_ms" } else { f };
        return Err(err(
            field(name),
            true,
            format!("{name} is for an edge entry (add edge: {{ from, to }})"),
        ));
    }
    Ok(Shaped {
        node: e.node.clone(),
        endpoint,
        replica: e.replica,
        edge: e.edge.as_ref().map(|x| (x.from.clone(), x.to.clone())),
        layer,
    })
}

/// **Private.** One pin after the topology-free checks.
pub(super) struct ShapedPin {
    pub(super) key: String,
    pub(super) entry: PinEntry,
}

/// **Private.** [`validate_scenario_shape`] keeping the checked entries.
pub(super) fn shape(cfg: &ScenarioConfig) -> Result<(Vec<Shaped>, Vec<ShapedPin>), ScenarioError> {
    cfg.check_supported().map_err(|m| {
        let field = m.split(':').next().unwrap_or_default().to_string();
        err(vec![PathSeg::Key(field)], false, m)
    })?;
    // The raw counts, before duplicate keys could collapse a huge list into a small one.
    for (name, n) in [("chaos", cfg.chaos.len()), ("pins", cfg.pins.len())] {
        if n > MAX_ENTRIES {
            return Err(err(
                vec![PathSeg::Key(name.into())],
                false,
                format!("scenario has {n} entries; at most {MAX_ENTRIES}"),
            ));
        }
    }
    if cfg.chaos.is_empty() && cfg.pins.is_empty() {
        return Err(err(
            vec![PathSeg::Key("chaos".into())],
            false,
            "scenario needs at least one of chaos or pins".into(),
        ));
    }
    let total = cfg.chaos.len() + cfg.pins.len();
    if total > MAX_ENTRIES {
        return Err(err(
            vec![PathSeg::Key("pins".into())],
            false,
            format!("scenario has {total} entries (chaos and pins); at most {MAX_ENTRIES}"),
        ));
    }
    let mut out = Vec::with_capacity(cfg.chaos.len());
    let mut first_at: HashMap<EntryKey, usize> = HashMap::new();
    for (i, e) in cfg.chaos.iter().enumerate() {
        let s = shape_entry(i, e)?;
        if let Some(j) = first_at.insert(
            (
                s.node.clone(),
                s.endpoint.clone(),
                s.replica,
                s.edge.clone(),
            ),
            i,
        ) {
            let at = vec![PathSeg::Key("chaos".into()), PathSeg::Index(i)];
            return Err(err(
                at,
                false,
                format!("entry has the same key as chaos[{j}]; merge them into one"),
            ));
        }
        out.push(s);
    }
    Ok((out, shape_pins(cfg)?))
}

/// **Private.** The topology-free checks of the pins: a well-formed endpoint, status and delay
/// in range for the state, no endpoint twice.
fn shape_pins(cfg: &ScenarioConfig) -> Result<Vec<ShapedPin>, ScenarioError> {
    let mut out: Vec<ShapedPin> = Vec::with_capacity(cfg.pins.len());
    for (i, p) in cfg.pins.iter().enumerate() {
        let field = |f: &str| {
            vec![
                PathSeg::Key("pins".into()),
                PathSeg::Index(i),
                PathSeg::Key(f.into()),
            ]
        };
        let key = endpoint_key(&p.endpoint).map_err(|m| err(field("endpoint"), true, m))?;
        check_entry(p.state, p.status, p.delay).map_err(|(f, m)| err(field(f), true, m))?;
        if let Some(j) = out.iter().position(|o| o.key == key) {
            return Err(err(
                vec![PathSeg::Key("pins".into()), PathSeg::Index(i)],
                false,
                format!(
                    "pins[{i}] pins '{key}' again; pins[{j}] already does, merge them into one"
                ),
            ));
        }
        out.push(ShapedPin {
            key,
            entry: PinEntry {
                state: p.state,
                status: p.status,
                delay: p.delay,
            },
        });
    }
    Ok(out)
}

/// Every check of a scenario that needs neither the topology nor the endpoints: the reserved
/// `limits` key, the entry counts, a scenario with neither `chaos` nor `pins`, per chaos entry
/// a node or endpoint, the endpoint's `METHOD /path` shape, the error kind, something set,
/// every value in range, per pin the endpoint shape and a status and delay valid for its
/// state, and duplicate keys. [`resolve_scenario`] runs it first; a client
/// (`kaanal scenario load`) runs it alone to place the error in the file.
///
/// # Errors
/// The first problem, placed by entry index and field.
pub fn validate_scenario_shape(cfg: &ScenarioConfig) -> Result<(), ScenarioError> {
    shape(cfg).map(|_| ())
}
