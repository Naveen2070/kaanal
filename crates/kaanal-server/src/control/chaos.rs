//! The chaos handlers: read and patch live chaos per node, endpoint or node at endpoint, and the topology view.

use kaanal_core::{EdgeRef, expected_visits};

use super::*;

/// **Private.** The public JSON shape of one chaos entry. `mode` shows only for an override.
#[derive(Serialize)]
pub(super) struct ChaosView {
    pub(super) latency_ms: u64,
    pub(super) error_rate: f64,
    pub(super) error_kind: &'static str,
    pub(super) error_status: u16,
    pub(super) down: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub(super) jitter_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bandwidth_cap: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) mode: Option<&'static str>,
}

impl From<&ChaosLayer> for ChaosView {
    /// Unset fields show their defaults (what the layer does on its own).
    fn from(l: &ChaosLayer) -> Self {
        let c = l.dense();
        ChaosView {
            latency_ms: u64::try_from(c.latency_add.as_millis()).unwrap_or(u64::MAX),
            error_rate: c.error_rate,
            error_kind: c.error_kind.as_str(),
            error_status: c.error_status,
            down: c.down,
            jitter_ms: l
                .jitter
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            bandwidth_cap: l.bandwidth_cap,
            mode: (l.replace == Some(true)).then_some("override"),
        }
    }
}

/// **Private.** One live entry's JSON: its [`ChaosView`] plus `expires_in_ms` (`{unit: ms}`)
/// when some of its units are leased.
pub(super) fn live_entry_json(
    l: &ChaosLayer,
    leases: &Leases,
    key: &ChaosKey,
    now: Instant,
) -> Value {
    let mut v = json!(ChaosView::from(l));
    let exp = chaos_lease::remaining(leases, key, now);
    if !exp.is_empty() {
        v["expires_in_ms"] = Value::Object(exp);
    }
    v
}

/// **Private.** `merge` / `override` to the layer's `replace` flag. Absent leaves the stored
/// value alone when patched in (`ChaosLayer::apply` skips `None`); `merge` stores `Some(false)`,
/// which clears an earlier override.
pub(super) fn parse_mode(mode: Option<&str>) -> Result<Option<bool>, Reject> {
    match mode {
        None => Ok(None),
        Some("merge") => Ok(Some(false)),
        Some("override") => Ok(Some(true)),
        Some(_) => Err(reject(
            StatusCode::BAD_REQUEST,
            "mode must be merge or override",
        )),
    }
}

/// The accepted body of `PUT /__kaanal/chaos/{node}`.
///
/// All fields are optional and merged into the existing settings;
/// unknown keys are rejected. `mode: "override"` makes the entry replace everything lower
/// layers (scenarios) contribute to that key; `"merge"` goes back to stacking. A PUT without
/// `mode` leaves the stored mode as it was.
///
/// `ttl` (`"30s"`, 100 ms to 24 h) leases every unit the patch sets (`latency`, `error`, `down`,
/// `mode`, see `chaos_lease::Unit`); without it, the units it sets are permanent. Units it does
/// not set keep their lease. A `ttl` with nothing to lease (`mode: merge` alone leases nothing) is a 400.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaosPatch {
    pub(super) latency_ms: Option<u64>,
    pub(super) error_rate: Option<f64>,
    pub(super) error_kind: Option<String>,
    pub(super) error_status: Option<u16>,
    pub(super) down: Option<bool>,
    pub(super) jitter_ms: Option<u64>,
    /// Bytes per second as a number, or text with a unit (`"500KB/s"`).
    #[serde(default, deserialize_with = "kaanal_core::units::bandwidth_cap")]
    pub(super) bandwidth_cap: Option<u64>,
    pub(super) mode: Option<String>,
    pub(super) ttl: Option<String>,
}

impl ChaosPatch {
    /// The sparse layer this patch sets.
    fn layer(&self) -> Result<ChaosLayer, Reject> {
        let error_kind = match &self.error_kind {
            None => None,
            Some(k) => Some(ErrorKind::parse(k).ok_or_else(|| {
                reject(
                    StatusCode::BAD_REQUEST,
                    "error_kind must be status, reset, hang or malformed",
                )
            })?),
        };
        Ok(ChaosLayer {
            latency_add: self.latency_ms.map(Duration::from_millis),
            error_rate: self.error_rate,
            error_kind,
            error_status: self.error_status,
            down: self.down,
            down_from: None,
            jitter: self.jitter_ms.map(Duration::from_millis),
            bandwidth_cap: self.bandwidth_cap,
            replace: parse_mode(self.mode.as_deref())?,
        })
    }
}

/// The accepted body of `PUT /__kaanal/chaos`: a [`ChaosPatch`] plus where it applies.
/// (Spelled out rather than `flatten`ed so unknown keys are still rejected.) Keep its patch fields in
/// step with [`ChaosPatch`]: a new chaos field must be added to both.
///
/// A `node` + `endpoint` pair is not checked against the endpoint's chain (a node it never visits just
/// never fires), and a `HEAD` route is a different key from its `GET`. `ttl` works as in
/// [`ChaosPatch`]: it leases the units the patch sets; units it does not set keep their lease.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyedPatch {
    pub(super) node: Option<String>,
    pub(super) endpoint: Option<String>,
    pub(super) replica: Option<u16>,
    pub(super) edge: Option<EdgeRef>,
    pub(super) latency_ms: Option<u64>,
    pub(super) error_rate: Option<f64>,
    pub(super) error_kind: Option<String>,
    pub(super) error_status: Option<u16>,
    pub(super) down: Option<bool>,
    pub(super) jitter_ms: Option<u64>,
    /// Bytes per second as a number, or text with a unit (`"500KB/s"`).
    #[serde(default, deserialize_with = "kaanal_core::units::bandwidth_cap")]
    pub(super) bandwidth_cap: Option<u64>,
    pub(super) mode: Option<String>,
    pub(super) ttl: Option<String>,
}

/// The query of `DELETE /__kaanal/chaos`. A misspelt key must be a 400, never "no filter" (which wipes all).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearQuery {
    pub(super) node: Option<String>,
    pub(super) endpoint: Option<String>,
    pub(super) replica: Option<u16>,
    pub(super) edge_from: Option<String>,
    pub(super) edge_to: Option<String>,
}

/// `GET /__kaanal/chaos` — the live layer
/// `{ "nodes": {name: view}, "endpoints": {key: view}, "node_at": [{node, endpoint, ...view}] }`
/// plus `"effective"`, the compiled result of the scenarios and the live layer.
pub async fn get_chaos(State(st): State<Arc<AppState>>) -> Result<Json<Value>, Reject> {
    // the lock pairs the chaos read below with the generation in force
    let (_c, g) = locked(&st)?;
    let snap = st.snapshot();
    let now = Instant::now();
    let items = snap
        .stack
        .live()
        .entries()
        .into_iter()
        .map(|(k, l)| {
            let v = live_entry_json(l, &snap.chaos_leases, &k, now);
            (k, v)
        })
        .collect();
    let mut out = keyed_json(&g, items);
    out.insert("effective".into(), effective_json(&g, &snap.chaos));
    Ok(Json(Value::Object(out)))
}

/// **Private.** Merges `patch` into the live layer under `key`, validates the merged layer
/// (limits live in core, R6.9) and publishes; a bad patch changes nothing. Returns the new
/// view plus `effective` and `warnings` (a kind or status without a rate, for this key only).
///
/// A patch with only `mode` stores a layer with no fields: `mode: override` alone calms the key
/// (it drops everything lower layers add and adds nothing). `mode: merge` on a key with no live
/// entry stores nothing.
///
/// The `ttl` is checked before anything changes. The patch's units are leased (or made permanent)
/// first; `commit` then prunes the leases of fields the write left unset (`down: false`).
pub(super) fn apply_patch(
    st: &AppState,
    g: &Generation,
    key: &ChaosKey,
    patch: &ChaosPatch,
) -> Result<Json<Value>, Reject> {
    let ttl = patch.ttl.as_deref().map(parse_ttl).transpose()?;
    let patch = patch.layer()?;
    let units = chaos_lease::units_set(&patch);
    // `mode: merge` counts as a unit but has nothing worth leasing (its expiry is a no-op).
    if ttl.is_some()
        && units.iter().all(|u| *u == chaos_lease::Unit::Mode)
        && patch.replace != Some(true)
    {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "ttl needs a field to lease",
        ));
    }
    let mut c = lock_gen(st, g)?;
    let snap = commit(st, &mut c, |c| {
        chaos_lease::apply(&mut c.chaos_leases, key, &units, ttl, Instant::now());
        let mut cur = c.stack.live().layer(key).cloned().unwrap_or_default();
        cur.apply(&patch);
        cur.validate()
            .map_err(|e| reject(StatusCode::BAD_REQUEST, &e))?;
        // link chaos belongs to an edge key, and an edge key has the link rules
        let linked = match key {
            ChaosKey::Edge(e) => cur.check_link(g.topo.edges[*e].bandwidth),
            _ => cur
                .link_only_field()
                .map_or(Ok(()), |f| Err((f, format!("{f} is for an edge key")))),
        };
        linked.map_err(|(_, m)| reject(StatusCode::BAD_REQUEST, &m))?;
        // `down: false` in the live layer means "unset": drop it, and drop an entry it leaves empty.
        if cur.down == Some(false) {
            cur.down = None;
        }
        if cur == ChaosLayer::default() {
            c.stack.live_mut().remove(key);
            return Ok(());
        }
        let bare_merge = cur.replace == Some(false)
            && ChaosLayer {
                replace: None,
                ..cur.clone()
            } == ChaosLayer::default();
        if !(bare_merge && c.stack.live().layer(key).is_none()) {
            c.stack.live_mut().set(key.clone(), cur);
        }
        Ok(())
    })?;
    drop(c);
    if ttl.is_some() {
        st.lease_wake.notify_one();
    }
    let stored = snap.stack.live().layer(key);
    let mut view = live_entry_json(
        &stored.cloned().unwrap_or_default(),
        &snap.chaos_leases,
        key,
        Instant::now(),
    );
    let warnings: Vec<String> = stored
        .filter(|l| l.has_orphan_fault_fields())
        .map(|_| {
            format!(
                "chaos on {}: error_kind/error_status have no effect without error_rate",
                key_label(g, key)
            )
        })
        .into_iter()
        .collect();
    view["effective"] = effective_json(g, &snap.chaos);
    view["warnings"] = json!(warnings);
    Ok(Json(view))
}

/// **Private.** The id of a node by name; unknown (or empty) → 404.
pub(super) fn node_id(g: &Generation, node: &str) -> Result<usize, Reject> {
    g.topo
        .id(node)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown node '{node}'")))
}

/// `GET /__kaanal/topology` — nodes, edges and sorted route keys as JSON.
pub async fn topology(Gen(g): Gen) -> Json<Value> {
    let t = &g.topo;
    let nodes: Vec<Value> = t
        .nodes
        .iter()
        .map(|n| {
            let mut v = json!({
                "name": n.name,
                "type": n.kind.as_str(),
                "flavor": n.flavor,
            });
            // only a replicated node says so, so a topology without replicas reads as before
            if n.replicas > 1 {
                v["replicas"] = json!(n.replicas);
            }
            v
        })
        .collect();
    let edges: Vec<Value> = t
        .edges
        .iter()
        .map(|e| json!({ "from": t.nodes[e.from].name, "to": t.nodes[e.to].name }))
        .collect();
    // Expected entries per request to each endpoint, per node (zero entries left out): what a
    // `prob`, a `repeat` or a cache hit does to the traffic a node sees.
    let endpoints: Vec<Value> = g
        .endpoints
        .iter()
        .filter(|ep| !ep.synthesized)
        .map(|ep| {
            let visits: Map<String, Value> = expected_visits(t, ep)
                .into_iter()
                .zip(&t.nodes)
                .filter(|(v, _)| *v > 0.0)
                .map(|(v, n)| (n.name.clone(), json!(v)))
                .collect();
            json!({ "key": ep.key, "visits": visits })
        })
        .collect();
    Json(json!({ "nodes": nodes, "edges": edges, "routes": g.route_keys, "endpoints": endpoints }))
}

/// `PUT /__kaanal/chaos/{node}` — merges a [`ChaosPatch`] into the node's live settings
/// (omitted fields keep their values) and returns the merged state, `effective` and `warnings`.
///
/// The merged layer is checked by [`ChaosLayer::validate`] (`error_rate` 0–1, `error_status`
/// 500–504 for every kind, `latency_ms` ≤ 300 000) before it is stored; a bad patch → 400 and
/// nothing changes. Unknown node → 404, bad body → 4xx from the extractor.
///
/// # Errors
/// Returns [`Reject`] (400/404 with a JSON `error` field) on invalid input.
pub async fn set_chaos(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    Path(node): Path<String>,
    payload: Result<Json<ChaosPatch>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let id = node_id(&g, &node)?;
    let Json(p) = payload.map_err(|e| reject(e.status(), &tidy(&e.body_text())))?;
    apply_patch(&st, &g, &ChaosKey::Node(id), &p)
}

/// **Private.** The key of a `replica` request, or `None` when none was named: a replica needs a
/// node, takes no endpoint (it applies whatever the endpoint) and must exist (400 otherwise).
fn replica_key(
    g: &Generation,
    node: Option<usize>,
    has_endpoint: bool,
    replica: Option<u16>,
) -> Result<Option<ChaosKey>, Reject> {
    let Some(r) = replica else { return Ok(None) };
    let bad = |m: &str| reject(StatusCode::BAD_REQUEST, m);
    let n = node.ok_or_else(|| bad("replica needs a node"))?;
    if has_endpoint {
        return Err(bad(
            "a replica takes no endpoint (it applies whatever the endpoint)",
        ));
    }
    let count = g.topo.nodes[n].replicas;
    if r >= count {
        return Err(bad(&format!(
            "node '{}' has {count} replicas (0 to {})",
            g.topo.nodes[n].name,
            count - 1
        )));
    }
    Ok(Some(ChaosKey::Replica(n, r)))
}

/// **Private.** The key of an `edge` request, or `None` when none was named: it takes no node,
/// endpoint or replica, both ends must be nodes (404) and the edge must exist (404).
fn edge_key(
    g: &Generation,
    other: bool,
    edge: Option<(&str, &str)>,
) -> Result<Option<ChaosKey>, Reject> {
    let Some((from, to)) = edge else {
        return Ok(None);
    };
    if other {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "an edge takes no node, endpoint or replica",
        ));
    }
    let id = node_id(g, from)?;
    let to_id = node_id(g, to)?;
    g.topo
        .edge_id(id, to_id)
        .map(|e| Some(ChaosKey::Edge(e)))
        .ok_or_else(|| {
            reject(
                StatusCode::NOT_FOUND,
                &format!("no edge '{from}' -> '{to}'"),
            )
        })
}

/// `PUT /__kaanal/chaos` — like [`set_chaos`] but keyed by `node`, `endpoint` or both (the node
/// only when visited through that endpoint). At least one is required; an unknown node or
/// endpoint → 404; a malformed endpoint (not `METHOD /path`) → 400; an endpoint with no entry
/// node (a static route and no gateway) → 400.
///
/// # Errors
/// Returns [`Reject`] (400/404 with a JSON `error` field) on invalid input.
pub async fn set_chaos_keyed(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    payload: Result<Json<KeyedPatch>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(p) = payload.map_err(|e| reject(e.status(), &tidy(&e.body_text())))?;
    let node = p.node.as_deref().map(|n| node_id(&g, n)).transpose()?;
    let endpoint = match p.endpoint.as_deref() {
        None => None,
        Some(e) => {
            let key = endpoint_key(e).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
            let ep = g.endpoints.get(&key).ok_or_else(|| {
                reject(StatusCode::NOT_FOUND, &format!("unknown endpoint '{key}'"))
            })?;
            if ep.entry_node.is_none() {
                return Err(reject(
                    StatusCode::BAD_REQUEST,
                    &format!("endpoint '{key}' has no entry node (a static route needs a gateway)"),
                ));
            }
            Some(key)
        }
    };
    let edge = p.edge.as_ref().map(|e| (e.from.as_str(), e.to.as_str()));
    let other = node.is_some() || endpoint.is_some() || p.replica.is_some();
    let key = if let Some(k) = edge_key(&g, other, edge)? {
        k
    } else if let Some(k) = replica_key(&g, node, p.endpoint.is_some(), p.replica)? {
        k
    } else {
        match (node, endpoint) {
            (Some(n), Some(e)) => ChaosKey::NodeAt(n, e),
            (Some(n), None) => ChaosKey::Node(n),
            (None, Some(e)) => ChaosKey::Endpoint(e),
            (None, None) => {
                return Err(reject(
                    StatusCode::BAD_REQUEST,
                    "set node, endpoint, edge or a node with an endpoint",
                ));
            }
        }
    };
    let patch = ChaosPatch {
        latency_ms: p.latency_ms,
        error_rate: p.error_rate,
        error_kind: p.error_kind,
        error_status: p.error_status,
        down: p.down,
        jitter_ms: p.jitter_ms,
        bandwidth_cap: p.bandwidth_cap,
        mode: p.mode,
        ttl: p.ttl,
    };
    apply_patch(&st, &g, &key, &patch)
}

/// `DELETE /__kaanal/chaos/{node}` — removes one node's live chaos (204). Scenarios stay.
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name.
pub async fn clear_node(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    Path(node): Path<String>,
) -> Result<StatusCode, Reject> {
    let id = node_id(&g, &node)?;
    let mut c = lock_gen(&st, &g)?;
    commit(&st, &mut c, |c| {
        c.stack.live_mut().remove(&ChaosKey::Node(id));
        Ok(())
    })?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /__kaanal/chaos?node=&endpoint=` — removes exactly the live entry under that key
/// (204, also when there was none); with no query, wipes the live layer (scenarios stay).
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name, 400 for a malformed endpoint (not `METHOD /path`).
pub async fn clear_keyed(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    q: Result<Query<ClearQuery>, QueryRejection>,
) -> Result<StatusCode, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let node = q.node.as_deref().map(|n| node_id(&g, n)).transpose()?;
    let endpoint = q
        .endpoint
        .as_deref()
        .map(endpoint_key)
        .transpose()
        .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let edge = q.edge_from.as_deref().zip(q.edge_to.as_deref());
    if q.edge_from.is_some() != q.edge_to.is_some() {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "an edge needs both edge_from and edge_to",
        ));
    }
    let other = node.is_some() || endpoint.is_some() || q.replica.is_some();
    let key = if let Some(k) = edge_key(&g, other, edge)? {
        Some(k)
    } else if let Some(k) = replica_key(&g, node, q.endpoint.is_some(), q.replica)? {
        Some(k)
    } else {
        match (node, endpoint) {
            (Some(n), Some(e)) => Some(ChaosKey::NodeAt(n, e)),
            (Some(n), None) => Some(ChaosKey::Node(n)),
            (None, Some(e)) => Some(ChaosKey::Endpoint(e)),
            (None, None) => None,
        }
    };
    let mut c = lock_gen(&st, &g)?;
    commit(&st, &mut c, |c| {
        match &key {
            Some(k) => {
                c.stack.live_mut().remove(k);
            }
            None => c.stack.live_mut().clear(),
        }
        Ok(())
    })?;
    Ok(StatusCode::NO_CONTENT)
}

/// **Private.** serde's `skip_serializing_if` for a zero count (a node view has no jitter).
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes a reference"
)]
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// **Private.** The extractor's text with a doubled field name collapsed: a unit-parse error names
/// its field and the extractor puts the path in front, so `bandwidth_cap: bandwidth_cap: ...`.
fn tidy(text: &str) -> String {
    text.replace("bandwidth_cap: bandwidth_cap: ", "bandwidth_cap: ")
}
