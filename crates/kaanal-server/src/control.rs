//! The control API handlers and the security guard that stands in front of them.
//!
//! Every `/__kaanal/*` request passes [`guard`] before any handler or extractor
//! runs: Host must be loopback with the bound port, and every route except
//! `health`/`ui` needs the bearer token.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, ErrorKind, MAX_SCENARIOS, PathSeg, Resolved,
    ScenarioConfig, Source, endpoint_key, parse_scenario, resolve_scenario, valid_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::Instant;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tokio_stream::{Stream, StreamExt};

use crate::chaos_lease::{self, Leases};
use crate::scenarios::{self, WriteError};
use crate::state::{AppState, BusEvent, Control, Effective};

/// **Private.** Uniform JSON error shape returned by handlers: `(status, {"error": msg})`.
pub(crate) type Reject = (StatusCode, Json<Value>);

// limitation: error messages echo the node name from the URL path (`unknown node '...'`) unsanitized.
// It is JSON-encoded, so the browser UI is safe, and the CLI replaces control characters before printing.
// Other clients that print it to a terminal must do the same.
pub(crate) fn reject(code: StatusCode, msg: &str) -> Reject {
    (code, Json(json!({ "error": msg })))
}

/// Runs in front of every `/__kaanal/*` route, before any extractor parses a body or query,
/// so unauthenticated callers never reach handler code:
/// 1. `Host` must be loopback with our port (DNS-rebinding guard), for the whole namespace.
/// 2. Everything except `health` and `ui` needs the bearer token. `/events` alone may take it
///    as `?token=`, because the browser `EventSource` cannot set headers.
/// 3. Responses are marked uncacheable and non-sniffable.
pub async fn guard(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host, st.port) {
        return reject(StatusCode::FORBIDDEN, "bad Host header").into_response();
    }
    let path = req.uri().path();
    if !matches!(path, "/__kaanal/health" | "/__kaanal/ui") {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));
        let query = if path == "/__kaanal/events" {
            query_token(req.uri().query())
        } else {
            None
        };
        match bearer.or(query) {
            Some(t) if !st.token.is_empty() && ct_eq(t.as_bytes(), st.token.as_bytes()) => {}
            _ => {
                return reject(StatusCode::UNAUTHORIZED, "missing or bad token").into_response();
            }
        }
    }
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}

/// **Private.** Extracts `token=` from a query string; `xtoken=` or a bare `token` do not match.
fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|kv| kv.strip_prefix("token="))
}

// limitation: by design (RULES R5.3). Even when `--host 0.0.0.0` exposes the server on the LAN,
// control calls must use `localhost`, `127.0.0.1` or `[::1]` as Host with the bound port. Reaching the
// control API by LAN IP or through a port-forward (container, SSH tunnel on another port) gets a 403.
// Add an explicit allowed-hosts setting if that is ever needed.
/// **Private.** True iff `host` is `localhost`/`127.0.0.1`/`[::1]` with no port or
/// the bound port. Case-insensitive; rejects `localhost.evil.example` and
/// `evil.example:8080@localhost`.
fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    let (name, p) = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((h, tail)) => (format!("[{h}]"), tail.strip_prefix(':')),
            None => return false,
        },
        None => match host.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (host.clone(), None),
        },
    };
    matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]")
        && p.is_none_or(|p| p.parse::<u16>().ok() == Some(port))
}

/// Constant-time comparison so the token can't be guessed byte by byte from timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// **Private.** The public JSON shape of one chaos entry. `mode` shows only for an override.
#[derive(Serialize)]
struct ChaosView {
    latency_ms: u64,
    error_rate: f64,
    error_kind: &'static str,
    error_status: u16,
    down: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'static str>,
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
            mode: (l.replace == Some(true)).then_some("override"),
        }
    }
}

/// **Private.** One live entry's JSON: its [`ChaosView`] plus `expires_in_ms` (`{unit: ms}`)
/// when some of its units are leased.
fn live_entry_json(l: &ChaosLayer, leases: &Leases, key: &ChaosKey, now: Instant) -> Value {
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
fn parse_mode(mode: Option<&str>) -> Result<Option<bool>, Reject> {
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
    latency_ms: Option<u64>,
    error_rate: Option<f64>,
    error_kind: Option<String>,
    error_status: Option<u16>,
    down: Option<bool>,
    mode: Option<String>,
    ttl: Option<String>,
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
    node: Option<String>,
    endpoint: Option<String>,
    latency_ms: Option<u64>,
    error_rate: Option<f64>,
    error_kind: Option<String>,
    error_status: Option<u16>,
    down: Option<bool>,
    mode: Option<String>,
    ttl: Option<String>,
}

/// The query of `DELETE /__kaanal/chaos`. A misspelt key must be a 400, never "no filter" (which wipes all).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearQuery {
    node: Option<String>,
    endpoint: Option<String>,
}

/// **Private.** The control state, locked. The guard must never live across an `.await`
/// (the handler futures would stop being `Send`, so the compiler enforces it).
///
/// A poisoned lock is recovered, not propagated: a panic in one handler must not take the whole
/// control plane down. `commit` restores its backup when the closure or the publish returns an
/// error, **not** when it panics: a panic mid-write leaves a half-written `Control`, which the
/// next successful write compiles and publishes. The request path is unaffected (it keeps the
/// last good published `Arc`); no handler closure panics in practice.
///
/// Only writers and the scenario GETs (`list_scenarios`, `get_scenario`, whose data lives in
/// the set) take this mutex. The request path never does: it reads the published `Arc`.
pub(crate) fn lock(st: &AppState) -> MutexGuard<'_, Control> {
    st.control.lock().unwrap_or_else(PoisonError::into_inner)
}

/// **Private.** The one write path (S11): runs `f` on the locked control state, publishes the
/// compiled result and returns it. If `f` or the publish fails, the state is put back, so a bad
/// request changes nothing.
pub(crate) fn commit(
    st: &AppState,
    c: &mut MutexGuard<'_, Control>,
    f: impl FnOnce(&mut Control) -> Result<(), Reject>,
) -> Result<Arc<Effective>, Reject> {
    let backup = (**c).clone();
    let done = f(c).and_then(|()| {
        // A lease outlives neither its activation nor its scenario, nor its live chaos field.
        let ctl = &mut **c;
        ctl.leases.retain(|n, _| ctl.stack.active().contains(n));
        chaos_lease::prune(&mut ctl.chaos_leases, ctl.stack.live());
        st.publish(c)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))
    });
    if done.is_err() {
        **c = backup;
    }
    done
}

/// **Private.** Tells SSE subscribers the stack changed.
pub(crate) fn emit(st: &AppState, action: &'static str, name: Option<&str>, snap: &Effective) {
    let _ = st.events.send(BusEvent::Scenario {
        action,
        name: name.map(str::to_string),
        chaos_key: None,
        active: snap.stack.active().to_vec(),
    });
}

/// **Private.** Places each view under its key: `{ nodes, endpoints, node_at }`.
fn keyed_json(st: &AppState, items: Vec<(ChaosKey, Value)>) -> Map<String, Value> {
    let (mut nodes, mut endpoints, mut node_at) = (Map::new(), Map::new(), Vec::new());
    for (k, view) in items {
        match k {
            ChaosKey::Node(id) => {
                nodes.insert(st.topo.nodes[id].name.clone(), view);
            }
            ChaosKey::Endpoint(e) => {
                endpoints.insert(e, view);
            }
            ChaosKey::NodeAt(id, e) => {
                let Value::Object(mut o) = view else { continue };
                o.insert("node".into(), json!(st.topo.nodes[id].name));
                o.insert("endpoint".into(), json!(e));
                node_at.push(Value::Object(o));
            }
        }
    }
    let mut out = Map::new();
    out.insert("nodes".into(), Value::Object(nodes));
    out.insert("endpoints".into(), Value::Object(endpoints));
    out.insert("node_at".into(), Value::Array(node_at));
    out
}

/// **Private.** One compiled key: `{ latency_ms, down, faults: [{upto, kind, status}] }`.
fn resolved_json(r: &Resolved) -> Value {
    let faults: Vec<Value> = r
        .faults()
        .iter()
        .map(|f| json!({ "upto": f.upto, "kind": f.kind.as_str(), "status": f.status }))
        .collect();
    json!({
        "latency_ms": u64::try_from(r.latency_add.as_millis()).unwrap_or(u64::MAX),
        "down": r.down,
        "faults": faults,
    })
}

/// **Private.** A compiled key that does nothing (a live override calmed it): not shown, not saved.
fn is_calm(r: &Resolved) -> bool {
    r.latency_add.is_zero() && !r.down && r.faults().is_empty()
}

/// **Private.** The compiled chaos as JSON; calm keys are left out.
fn effective_json(st: &AppState, e: &EffectiveChaos) -> Value {
    let items = e
        .entries()
        .into_iter()
        .filter(|(_, r)| !is_calm(r))
        .map(|(k, r)| (k, resolved_json(r)))
        .collect();
    Value::Object(keyed_json(st, items))
}

/// **Private.** `{ active, effective }` of a published snapshot.
fn active_json(st: &AppState, snap: &Effective) -> Value {
    let now = Instant::now();
    let expires: Map<String, Value> = snap
        .leases
        .iter()
        .map(|(n, d)| (n.clone(), json!(remaining_ms(*d, now))))
        .collect();
    json!({
        "active": snap.stack.active(),
        "expires_in_ms": expires,
        "effective": effective_json(st, &snap.chaos),
    })
}

/// **Private.** Milliseconds from `now` to `deadline`, rounded up (0 once past).
pub(crate) fn remaining_ms(deadline: Instant, now: Instant) -> u64 {
    let ns = deadline.saturating_duration_since(now).as_nanos();
    u64::try_from(ns.div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// `GET /__kaanal/chaos` — the live layer
/// `{ "nodes": {name: view}, "endpoints": {key: view}, "node_at": [{node, endpoint, ...view}] }`
/// plus `"effective"`, the compiled result of the scenarios and the live layer.
pub async fn get_chaos(State(st): State<Arc<AppState>>) -> Json<Value> {
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
    let mut out = keyed_json(&st, items);
    out.insert("effective".into(), effective_json(&st, &snap.chaos));
    Json(Value::Object(out))
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
fn apply_patch(st: &AppState, key: &ChaosKey, patch: &ChaosPatch) -> Result<Json<Value>, Reject> {
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
    let mut c = lock(st);
    let snap = commit(st, &mut c, |c| {
        chaos_lease::apply(&mut c.chaos_leases, key, &units, ttl, Instant::now());
        let mut cur = c.stack.live().layer(key).cloned().unwrap_or_default();
        cur.apply(&patch);
        cur.validate()
            .map_err(|e| reject(StatusCode::BAD_REQUEST, &e))?;
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
                key_label(st, key)
            )
        })
        .into_iter()
        .collect();
    view["effective"] = effective_json(st, &snap.chaos);
    view["warnings"] = json!(warnings);
    Ok(Json(view))
}

/// **Private.** The id of a node by name; unknown (or empty) → 404.
fn node_id(st: &AppState, node: &str) -> Result<usize, Reject> {
    st.topo
        .id(node)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown node '{node}'")))
}

/// `GET /__kaanal/topology` — nodes, edges and sorted route keys as JSON.
pub async fn topology(State(st): State<Arc<AppState>>) -> Json<Value> {
    let t = &st.topo;
    let nodes: Vec<Value> = t
        .nodes
        .iter()
        .map(|n| {
            json!({
                "name": n.name,
                "type": n.kind.as_str(),
                "flavor": n.flavor,
            })
        })
        .collect();
    let edges: Vec<Value> = t
        .edges
        .iter()
        .map(|e| json!({ "from": t.nodes[e.from].name, "to": t.nodes[e.to].name }))
        .collect();
    Json(json!({ "nodes": nodes, "edges": edges, "routes": st.route_keys }))
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
    Path(node): Path<String>,
    payload: Result<Json<ChaosPatch>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let id = node_id(&st, &node)?;
    let Json(p) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    apply_patch(&st, &ChaosKey::Node(id), &p)
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
    payload: Result<Json<KeyedPatch>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(p) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let node = p.node.as_deref().map(|n| node_id(&st, n)).transpose()?;
    let endpoint = match p.endpoint.as_deref() {
        None => None,
        Some(e) => {
            let key = endpoint_key(e).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
            let ep = st.endpoints.get(&key).ok_or_else(|| {
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
    let key = match (node, endpoint) {
        (Some(n), Some(e)) => ChaosKey::NodeAt(n, e),
        (Some(n), None) => ChaosKey::Node(n),
        (None, Some(e)) => ChaosKey::Endpoint(e),
        (None, None) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "set node, endpoint or both",
            ));
        }
    };
    let patch = ChaosPatch {
        latency_ms: p.latency_ms,
        error_rate: p.error_rate,
        error_kind: p.error_kind,
        error_status: p.error_status,
        down: p.down,
        mode: p.mode,
        ttl: p.ttl,
    };
    apply_patch(&st, &key, &patch)
}

/// `DELETE /__kaanal/chaos/{node}` — removes one node's live chaos (204). Scenarios stay.
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name.
pub async fn clear_node(
    State(st): State<Arc<AppState>>,
    Path(node): Path<String>,
) -> Result<StatusCode, Reject> {
    let id = node_id(&st, &node)?;
    let mut c = lock(&st);
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
    q: Result<Query<ClearQuery>, QueryRejection>,
) -> Result<StatusCode, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let node = q.node.as_deref().map(|n| node_id(&st, n)).transpose()?;
    let endpoint = q
        .endpoint
        .as_deref()
        .map(endpoint_key)
        .transpose()
        .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let key = match (node, endpoint) {
        (Some(n), Some(e)) => Some(ChaosKey::NodeAt(n, e)),
        (Some(n), None) => Some(ChaosKey::Node(n)),
        (None, Some(e)) => Some(ChaosKey::Endpoint(e)),
        (None, None) => None,
    };
    let mut c = lock(&st);
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

/// **Private.** `inline`, `file` or `runtime`.
fn source_name(s: Source) -> &'static str {
    match s {
        Source::Inline => "inline",
        Source::File => "file",
        Source::Runtime => "runtime",
    }
}

/// **Private.** Adds the key's `node` / `endpoint` fields to an entry.
fn key_fields(st: &AppState, k: &ChaosKey, o: &mut Map<String, Value>) {
    match k {
        ChaosKey::Node(id) => {
            o.insert("node".into(), json!(st.topo.nodes[*id].name));
        }
        ChaosKey::Endpoint(e) => {
            o.insert("endpoint".into(), json!(e));
        }
        ChaosKey::NodeAt(id, e) => {
            o.insert("node".into(), json!(st.topo.nodes[*id].name));
            o.insert("endpoint".into(), json!(e));
        }
    }
}

/// **Private.** For a message: `node 'db' at endpoint 'GET /x'`.
fn key_label(st: &AppState, k: &ChaosKey) -> String {
    match k {
        ChaosKey::Node(id) => format!("node '{}'", st.topo.nodes[*id].name),
        ChaosKey::Endpoint(e) => format!("endpoint '{e}'"),
        ChaosKey::NodeAt(id, e) => format!("node '{}' at endpoint '{e}'", st.topo.nodes[*id].name),
    }
}

/// **Private.** A sparse state as scenario entries: only the fields that are set (never the
/// dense defaults), so reloading a saved file adds no explicit value that would last-win.
/// A layer that sets nothing (a bare `mode: merge`) is skipped.
fn layers_json(st: &AppState, layers: &ChaosState) -> Vec<Value> {
    let mut out = Vec::new();
    for (k, l) in layers.entries() {
        let bare = ChaosLayer {
            replace: None,
            ..l.clone()
        };
        if bare == ChaosLayer::default() {
            continue;
        }
        let mut o = Map::new();
        key_fields(st, &k, &mut o);
        if let Some(d) = l.latency_add {
            o.insert(
                "latency_ms".into(),
                json!(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if let Some(r) = l.error_rate {
            o.insert("error_rate".into(), json!(r));
        }
        if let Some(kind) = l.error_kind {
            o.insert("error_kind".into(), json!(kind.as_str()));
        }
        if let Some(s) = l.error_status {
            o.insert("error_status".into(), json!(s));
        }
        if let Some(d) = l.down {
            o.insert("down".into(), json!(d));
        }
        out.push(Value::Object(o));
    }
    out
}

/// **Private.** The compiled chaos as scenario entries (`save` with `from: effective`).
/// Limit: a key with two or more failure bands has no single-entry form, and a node@endpoint
/// pair is compiled together with its node's own chaos (writing both would count the node
/// twice); both are refused with a message naming the key. A pair that compiled to exactly
/// its node's own chaos adds nothing and is skipped rather than refused.
fn effective_entries(st: &AppState, e: &EffectiveChaos) -> Result<Vec<Value>, Reject> {
    let all = e.entries();
    let nodes: HashMap<usize, &Resolved> = all
        .iter()
        .filter_map(|(k, r)| match k {
            ChaosKey::Node(id) if !is_calm(r) => Some((*id, *r)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for (k, r) in all {
        if let ChaosKey::NodeAt(id, _) = &k
            && let Some(own) = nodes.get(id)
        {
            if r == *own {
                continue;
            }
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "{} is compiled together with the node's own chaos and cannot be written alone; save from live or from a scenario",
                    key_label(st, &k)
                ),
            ));
        }
        if r.faults().len() > 1 {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "{} has {} failure bands, which one entry cannot hold; save from live or from a scenario",
                    key_label(st, &k),
                    r.faults().len()
                ),
            ));
        }
        let mut o = Map::new();
        key_fields(st, &k, &mut o);
        let keyed = o.len();
        if !r.latency_add.is_zero() {
            o.insert(
                "latency_ms".into(),
                json!(u64::try_from(r.latency_add.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if r.down {
            o.insert("down".into(), json!(true));
        }
        if let Some(f) = r.faults().first() {
            o.insert("error_rate".into(), json!(f.upto));
            o.insert("error_kind".into(), json!(f.kind.as_str()));
            o.insert("error_status".into(), json!(f.status));
        }
        if o.len() > keyed {
            out.push(Value::Object(o));
        }
    }
    Ok(out)
}

/// **Private.** A scenario error as `chaos[2].node: message`.
fn place(path: &[PathSeg], msg: &str) -> String {
    let mut s = String::new();
    for p in path {
        match p {
            PathSeg::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            PathSeg::Index(i) => {
                let _ = write!(s, "[{i}]");
            }
        }
    }
    if s.is_empty() {
        msg.to_string()
    } else {
        format!("{s}: {msg}")
    }
}

/// **Private.** 404 for a scenario name.
fn unknown(name: &str) -> Reject {
    reject(StatusCode::NOT_FOUND, &format!("unknown scenario '{name}'"))
}

/// `GET /__kaanal/scenarios` — every defined scenario with its source, entry count and whether
/// it is active.
pub async fn list_scenarios(State(st): State<Arc<AppState>>) -> Json<Value> {
    // Locked first, so the set and the active stack are from the same write.
    let c = lock(&st);
    let snap = st.snapshot();
    let now = Instant::now();
    let out: Vec<Value> = c
        .scenarios
        .names()
        .into_iter()
        .filter_map(|n| c.scenarios.get(n))
        .map(|(s, src)| {
            let mut o = json!({
                "name": s.name,
                "source": source_name(src),
                "description": s.description,
                "entries": s.layers.entry_count() + s.pins.len(),
                "active": snap.stack.active().contains(&s.name),
            });
            o["pins"] = json!(pins_json(&s.pins));
            if let Some(d) = snap.leases.get(&s.name) {
                o["expires_in_ms"] = json!(remaining_ms(*d, now));
            }
            o
        })
        .collect();
    Json(Value::Array(out))
}

/// `GET /__kaanal/scenarios/{name}` — one definition.
///
/// # Errors
/// 404 for an unknown name.
pub async fn get_scenario(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, Reject> {
    let c = lock(&st);
    let (s, src) = c.scenarios.get(&name).ok_or_else(|| unknown(&name))?;
    Ok(Json(json!({
        "name": s.name,
        "source": source_name(src),
        "description": s.description,
        "chaos": layers_json(&st, &s.layers),
        "pins": pins_json(&s.pins),
    })))
}

/// **Private.** A scenario's pins as the entries a `PUT` accepts.
pub(crate) fn pins_json(pins: &kaanal_core::PinLayer) -> Vec<Value> {
    pins.iter()
        .map(|(k, e)| {
            let mut o = json!({ "endpoint": k, "state": e.state.name() });
            if let Some(s) = e.status {
                o["status"] = json!(s);
            }
            if let Some(d) = e.delay {
                o["delay"] = json!(humantime::format_duration(d).to_string());
            }
            o
        })
        .collect()
}

/// `PUT /__kaanal/scenarios/{name}` — defines or replaces a runtime scenario. The body is a
/// scenario file (`description?`, `chaos: [...]`) checked exactly like one: names to ids,
/// endpoints through `endpoint_key`. Replacing an active one re-compiles. Answers
/// `{ name, warnings }`.
///
/// # Errors
/// 400 for a bad name or entry, 404 for an unknown node or endpoint, 409 when the name is
/// defined in the config or a file.
pub async fn put_scenario(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
    payload: Result<Json<ScenarioConfig>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    valid_name(&name).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let Json(cfg) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let (scenario, warnings) = resolve_scenario(&name, &cfg, &st.topo, &st.endpoints, &|ep| {
        st.empty_ok_for(ep)
    })
    .map_err(|e| {
        let code = if e.not_found {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        reject(code, &place(&e.path, &e.message))
    })?;
    let mut c = lock(&st);
    let was_active = c.stack.active().contains(&name);
    let snap = commit(&st, &mut c, |c| {
        if let Some((_, src)) = c.scenarios.get(&name)
            && src != Source::Runtime
        {
            return Err(reject(
                StatusCode::CONFLICT,
                &format!(
                    "scenario '{name}' comes from the config or a file; it cannot be redefined at runtime"
                ),
            ));
        }
        c.scenarios
            .replace(scenario)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))
    })?;
    if was_active {
        // The effective chaos changed under a running stack.
        emit(&st, "update", Some(&name), &snap);
    }
    Ok(Json(json!({ "name": name, "warnings": warnings })))
}

/// `DELETE /__kaanal/scenarios/{name}` — undefines a runtime scenario (S19) and drops it from
/// the stack when active.
///
/// # Errors
/// 404 for an unknown name, 409 for an inline or file-defined one.
pub async fn delete_scenario(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, Reject> {
    let mut c = lock(&st);
    let was_active = c.stack.active().contains(&name);
    let snap = commit(&st, &mut c, |c| {
        match c.scenarios.get(&name) {
            None => return Err(unknown(&name)),
            Some((_, Source::Runtime)) => {}
            Some(_) => {
                return Err(reject(
                    StatusCode::CONFLICT,
                    &format!(
                        "scenario '{name}' comes from the config or a file and cannot be undefined at runtime"
                    ),
                ));
            }
        }
        c.scenarios
            .remove(&name)
            .map_err(|m| reject(StatusCode::CONFLICT, &m))?;
        c.stack.prune(&c.scenarios);
        Ok(())
    })?;
    if was_active {
        emit(&st, "deactivate", Some(&name), &snap);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /__kaanal/scenarios/{name}/activate` — pushes (or moves) the scenario to the top of
/// the stack. Answers `{ active, expires_in_ms, effective }`.
///
/// An empty body is a plain activation (it also removes any lease the scenario had). A body
/// `{ "ttl": "30s" }` (100 ms to 24 h, humantime) leases it: when the time is up the scenario is
/// deactivated and an `expire` event is sent. Activating again replaces the lease.
///
/// # Errors
/// 400 for a bad body or ttl, 404 for an unknown name.
pub async fn activate(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let ttl = if body.is_empty() {
        None
    } else {
        let b: ActivateBody = serde_json::from_slice(&body)
            .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
        b.ttl.as_deref().map(parse_ttl).transpose()?
    };
    let mut c = lock(&st);
    let snap = commit(&st, &mut c, |c| {
        c.stack
            .activate(&c.scenarios, &name)
            .map_err(|_| unknown(&name))?;
        match ttl {
            Some(t) => c.leases.insert(name.clone(), Instant::now() + t),
            None => c.leases.remove(&name),
        };
        Ok(())
    })?;
    emit(&st, "activate", Some(&name), &snap);
    drop(c);
    st.lease_wake.notify_one();
    Ok(Json(active_json(&st, &snap)))
}

/// The accepted body of `POST /__kaanal/scenarios/{name}/activate`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateBody {
    ttl: Option<String>,
}

/// **Private.** A `ttl` string such as `30s`, within 100 ms to 24 h.
fn parse_ttl(s: &str) -> Result<Duration, Reject> {
    humantime::parse_duration(s)
        .ok()
        .filter(|d| (Duration::from_millis(100)..=Duration::from_hours(24)).contains(d))
        .ok_or_else(|| {
            reject(
                StatusCode::BAD_REQUEST,
                "ttl must be 100ms to 24h, e.g. \"30s\"",
            )
        })
}

/// Deactivates every scenario whose lease deadline has passed, one `expire` event each, earliest
/// first; then clears the due live chaos units (see [`expire_chaos`]). Returns how many expiries
/// it published: one per scenario, one per chaos key. Every deadline is compared under the lock,
/// so a lease replaced between the task's wake-up and here is not expired early.
///
/// No past-due lease is left behind, whatever happens (the task would spin on it).
pub fn expire_due(st: &AppState) -> usize {
    let mut c = lock(st);
    let now = Instant::now();
    let mut due: Vec<(Instant, String)> = c
        .leases
        .iter()
        .filter(|(_, d)| **d <= now)
        .map(|(n, d)| (*d, n.clone()))
        .collect();
    due.sort();
    let mut n = 0;
    for (_, name) in due {
        // `commit` drops the lease with the activation.
        let done = commit(st, &mut c, |c| {
            c.stack.deactivate(&name);
            Ok(())
        });
        if let Ok(snap) = done {
            emit(st, "expire", Some(&name), &snap);
            n += 1;
        } else {
            // The restore put the past-due lease back; left there, `lease_task` would spin.
            // Drop only the lease: the scenario stays active, so Control and the published
            // state still agree, and a manual deactivate works through `commit`.
            eprintln!("kaanal: could not expire '{name}'; left it active and dropped its lease");
            c.leases.remove(&name);
        }
    }
    n + expire_chaos(st, &mut c)
}

/// **Private.** The chaos half of [`expire_due`] (spec L3): the due units are grouped by key, and
/// each key gets one `commit` that clears them from its live entry (an entry left empty is
/// removed), so one publish and one `expire` event (`chaos_key` set, no `name`) per key. A failed
/// commit leaves the chaos in place and drops the leases. A lease with no unit behind it (only
/// possible if a write skipped `commit`) is dropped without an event.
fn expire_chaos(st: &AppState, c: &mut MutexGuard<'_, Control>) -> usize {
    let now = Instant::now();
    let mut due: Vec<(Instant, ChaosKey, chaos_lease::Unit)> = c
        .chaos_leases
        .iter()
        .filter(|(_, d)| **d <= now)
        .map(|((k, u), d)| (*d, k.clone(), *u))
        .collect();
    due.sort_by_key(|(d, _, u)| (*d, *u));
    let mut keys: Vec<(ChaosKey, Vec<chaos_lease::Unit>)> = Vec::new();
    for (_, k, u) in due {
        match keys.iter_mut().find(|(have, _)| *have == k) {
            Some((_, us)) => us.push(u),
            None => keys.push((k, vec![u])),
        }
    }
    let mut n = 0;
    for (key, units) in keys {
        let set = c
            .stack
            .live()
            .layer(&key)
            .map(chaos_lease::units_set)
            .unwrap_or_default();
        if !units.iter().any(|u| set.contains(u)) {
            for u in &units {
                c.chaos_leases.remove(&(key.clone(), *u));
            }
            continue;
        }
        // `commit` drops the leases with their fields.
        let done = commit(st, c, |c| {
            let mut l = c.stack.live().layer(&key).cloned().unwrap_or_default();
            for u in &units {
                chaos_lease::clear_units(&mut l, *u);
                c.chaos_leases.remove(&(key.clone(), *u));
            }
            if l == ChaosLayer::default() {
                c.stack.live_mut().remove(&key);
            } else {
                c.stack.live_mut().set(key.clone(), l);
            }
            Ok(())
        });
        if let Ok(snap) = done {
            let mut k = Map::new();
            key_fields(st, &key, &mut k);
            let _ = st.events.send(BusEvent::Scenario {
                action: "expire",
                name: None,
                chaos_key: Some(Value::Object(k)),
                active: snap.stack.active().to_vec(),
            });
            n += 1;
        } else {
            // As for scenarios: the restore put the past-due leases back; drop only them.
            let label = key_label(st, &key);
            eprintln!(
                "kaanal: could not expire chaos on {label}; left it in place and dropped its lease"
            );
            for u in &units {
                c.chaos_leases.remove(&(key.clone(), *u));
            }
        }
    }
    n
}

/// **Private.** The earliest deadline over scenario and chaos leases (spec L4: one timer).
fn next_deadline(c: &Control) -> Option<Instant> {
    c.leases
        .values()
        .chain(c.chaos_leases.values())
        .min()
        .copied()
}

/// Expires leases as their deadlines pass; spawned once by `serve`. Sleeps to the earliest
/// deadline of both tables, or until a write adds or replaces a lease.
///
/// No wake-up is lost: `notify_one` stores a permit when nobody waits, so a lease added between
/// reading the deadline and polling `notified()` makes that `notified()` return at once.
pub async fn lease_task(st: Arc<AppState>) {
    loop {
        let next = next_deadline(&lock(&st));
        match next {
            None => st.lease_wake.notified().await,
            Some(d) => tokio::select! {
                () = tokio::time::sleep_until(d) => { expire_due(&st); }
                () = st.lease_wake.notified() => {}
            },
        }
    }
}

/// `POST /__kaanal/scenarios/{name}/deactivate` — removes it from the stack (204; also when it
/// was not active).
pub async fn deactivate(State(st): State<Arc<AppState>>, Path(name): Path<String>) -> StatusCode {
    let mut c = lock(&st);
    if !c.stack.active().contains(&name) {
        return StatusCode::NO_CONTENT;
    }
    if let Ok(snap) = commit(&st, &mut c, |c| {
        c.stack.deactivate(&name);
        Ok(())
    }) {
        emit(&st, "deactivate", Some(&name), &snap);
    }
    StatusCode::NO_CONTENT
}

/// `PUT /__kaanal/scenarios/active` — body `["a","b"]` sets the whole stack (bottom to top),
/// all or nothing. Answers like `activate`.
///
/// # Errors
/// 404 for an unknown name; nothing changes.
pub async fn set_active(
    State(st): State<Arc<AppState>>,
    payload: Result<Json<Vec<String>>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(names) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let mut c = lock(&st);
    let snap = commit(&st, &mut c, |c| {
        c.leases.clear();
        c.stack
            .set_active(&c.scenarios, &names)
            .map_err(|m| reject(StatusCode::NOT_FOUND, &m))
    })?;
    emit(&st, "set", None, &snap);
    Ok(Json(active_json(&st, &snap)))
}

/// `GET /__kaanal/scenarios/active` — the stack and the compiled result.
pub async fn get_active(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(active_json(&st, &st.snapshot()))
}

/// `DELETE /__kaanal/scenarios` — deactivates every scenario; the live layer stays (S12).
///
/// # Errors
/// Never in practice (the combined chaos was already valid).
pub async fn clear_scenarios(State(st): State<Arc<AppState>>) -> Result<StatusCode, Reject> {
    let mut c = lock(&st);
    let snap = commit(&st, &mut c, |c| {
        c.stack.clear_scenarios();
        Ok(())
    })?;
    emit(&st, "clear", None, &snap);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /__kaanal/reset` — back to a clean slate for `beforeEach` (S18): deactivates every
/// scenario, **undefines the runtime-defined ones** (inline and file scenarios stay), clears the
/// live layer, zeroes every route counter and refills the engine's rate-limit buckets, so a
/// seeded run repeats.
///
/// Requests already in flight are not interrupted: they keep the chaos they were planned with
/// and the slots and queue places they hold (gates and pools are deliberately not reset, which
/// would leak or double-free a slot; they drain as those requests finish). A request that
/// races the reset may take a counter value or a bucket token on either side of it; reset
/// between test cases, not during one. The counters and buckets are touched only after the
/// publish succeeded, so a failed reset changes nothing.
///
/// # Errors
/// Never in practice.
pub async fn reset(State(st): State<Arc<AppState>>) -> Result<StatusCode, Reject> {
    let mut c = lock(&st);
    let snap = commit(&st, &mut c, |c| {
        c.stack.clear_all();
        let runtime: Vec<String> = c
            .scenarios
            .names()
            .into_iter()
            .filter(|n| {
                c.scenarios
                    .get(n)
                    .is_some_and(|(_, s)| s == Source::Runtime)
            })
            .map(str::to_string)
            .collect();
        for n in runtime {
            let _ = c.scenarios.remove(&n);
        }
        Ok(())
    })?;
    for n in &st.counters {
        n.store(0, Ordering::Relaxed);
    }
    if let Some(e) = &st.engine {
        e.refill_buckets();
    }
    st.log
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    emit(&st, "reset", None, &snap);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /__kaanal/shutdown` — starts a graceful stop. Body (optional): `{ "timeout": "5s" }`,
/// 1 ms to 300 s, humantime; no body means 5 s. In-flight requests get that long to finish, then
/// the connections still open are cut. Answers `202 { shutting_down, timeout_ms }`; a second
/// call changes nothing and reports the time left. A body of only whitespace counts as no body.
///
/// The deadline is fixed before this answer is flushed, so a tiny budget can cut the connection
/// before the `202` reaches the caller; a client that sees a transport error here, with the server
/// lock held (`down`), should read it as "the drain is running". Open `/events` streams end at
/// once with a final `shutdown` message instead of holding the drain.
///
/// # Errors
/// 400 for a body that is not a JSON object, an unknown key, or a bad `timeout`.
pub async fn shutdown(
    State(st): State<Arc<AppState>>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), Reject> {
    let bad = |m: &str| reject(StatusCode::BAD_REQUEST, m);
    let timeout = if body.trim_ascii().is_empty() {
        None
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Object(mut o)) => {
                let t = o.remove("timeout");
                if let Some(k) = o.keys().next() {
                    return Err(bad(&format!("unknown field '{k}'")));
                }
                t
            }
            Ok(_) => return Err(bad("body must be a JSON object, e.g. {\"timeout\":\"5s\"}")),
            Err(e) => return Err(bad(&format!("bad body: {e}"))),
        }
    };
    let budget = match timeout {
        None => Duration::from_secs(5),
        Some(Value::String(s)) => humantime::parse_duration(&s)
            .ok()
            .filter(|d| (Duration::from_millis(1)..=Duration::from_secs(300)).contains(d))
            .ok_or_else(|| bad("timeout must be 1ms to 300s, e.g. \"5s\""))?,
        Some(_) => return Err(bad("timeout must be a duration string, e.g. \"5s\"")),
    };
    let left = st.shutdown.request(budget);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "shutting_down": true,
            "timeout_ms": u64::try_from(left.as_millis()).unwrap_or(u64::MAX),
        })),
    ))
}

/// Where `save` takes its entries from: `"live"`, `"effective"` or `{ "scenario": "<name>" }`.
/// (Tagged, so a scenario may itself be called `live` or `effective`.)
///
/// Strict: serde's own enum would also take `{"live": null}`; this takes exactly the three forms.
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub enum SaveFrom {
    /// The live layer.
    Live,
    /// The compiled result of the stack and the live layer.
    Effective,
    /// A defined scenario.
    Scenario(String),
}

impl TryFrom<Value> for SaveFrom {
    type Error = String;

    fn try_from(v: Value) -> Result<Self, String> {
        const HINT: &str = "from must be \"live\", \"effective\" or {\"scenario\": \"<name>\"}";
        match &v {
            Value::String(s) if s == "live" => Ok(SaveFrom::Live),
            Value::String(s) if s == "effective" => Ok(SaveFrom::Effective),
            Value::Object(o) if o.len() == 1 => match o.get("scenario") {
                Some(Value::String(n)) => Ok(SaveFrom::Scenario(n.clone())),
                _ => Err(HINT.into()),
            },
            _ => Err(HINT.into()),
        }
    }
}

/// The accepted body of `POST /__kaanal/scenarios/save`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveBody {
    name: String,
    from: Option<SaveFrom>,
    force: Option<bool>,
    clear_live: Option<bool>,
}

/// **Private.** A scenario file's text. JSON is YAML: one flow mapping per entry, strings
/// escaped by `serde_json`.
fn scenario_yaml(description: Option<&str>, chaos: &[Value], pins: &[Value]) -> String {
    let mut yaml = String::new();
    if let Some(d) = description {
        let _ = writeln!(yaml, "description: {}", Value::String(d.to_string()));
    }
    for (head, list) in [("chaos", chaos), ("pins", pins)] {
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(yaml, "{head}:");
        for e in list {
            let _ = writeln!(yaml, "  - {e}");
        }
    }
    yaml
}

/// **Private.** What a save writes: the chaos entries, the pin entries and a description.
struct SaveSource {
    chaos: Vec<Value>,
    pins: Vec<Value>,
    description: Option<String>,
}

/// **Private.** The [`SaveSource`] of a save's `from`. Runs under the control lock, so
/// `effective` is the latest published state.
fn save_source(
    st: &AppState,
    c: &MutexGuard<'_, Control>,
    from: SaveFrom,
) -> Result<SaveSource, Reject> {
    let (chaos, pins, description) = match from {
        SaveFrom::Live => {
            if let Some((k, _)) = c
                .stack
                .live()
                .entries()
                .into_iter()
                .find(|(_, l)| l.replace == Some(true))
            {
                return Err(reject(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "the live chaos on {} is an override, which a scenario file cannot hold; save from effective",
                        key_label(st, &k)
                    ),
                ));
            }
            (
                layers_json(st, c.stack.live()),
                pins_json(c.stack.live_pins()),
                None,
            )
        }
        // Under the lock, so this is the latest published state.
        SaveFrom::Effective => {
            let snap = st.snapshot();
            let mut won = kaanal_core::PinLayer::default();
            for (k, r) in snap.pins.iter() {
                won.insert(k.clone(), r.entry);
            }
            (effective_entries(st, &snap.chaos)?, pins_json(&won), None)
        }
        SaveFrom::Scenario(other) => match c.scenarios.get(&other) {
            None => return Err(unknown(&other)),
            Some((s, _)) => (
                layers_json(st, &s.layers),
                pins_json(&s.pins),
                s.description.clone(),
            ),
        },
    };
    Ok(SaveSource {
        chaos,
        pins,
        description,
    })
}

/// `POST /__kaanal/scenarios/save` — writes `scenarios/NAME.yaml` (S17) from the live layer
/// (default), the compiled `effective` result, or another scenario, and registers it as a file
/// scenario. Sparse: only the fields that were set. Answers `{ name, path, warnings }`; from
/// live or effective, `warnings` counts the leased fields the file makes permanent.
///
/// Limits: a live override cannot be saved (files are merge-only; use `from: effective`), and
/// `from: effective` refuses a key with two or more failure bands or a node@endpoint pair that
/// shares a node with node-level chaos (see `effective_entries`). `from` is `"live"` (default),
/// `"effective"` or `{ "scenario": "<name>" }`; any other `from` is a 422 from the extractor.
/// Saving over an active scenario emits an `update`
/// event. Directory use is check-then-use: the parent is verified just before the write, so a
/// directory swapped in that instant is not caught (the local user owns the directory anyway).
///
/// # Errors
/// 400 for a bad name, an inline name or nothing to save; 404 for an unknown `from`; 409 when
/// the scenario or its file exists and `force` is not set.
pub async fn save_scenario(
    State(st): State<Arc<AppState>>,
    payload: Result<Json<SaveBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    valid_name(&b.name).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let force = b.force.unwrap_or(false);
    let mut c = lock(&st);
    match c.scenarios.get(&b.name) {
        Some((_, Source::Inline)) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "scenario '{}' is defined in kaanal.yaml; pick another name",
                    b.name
                ),
            ));
        }
        Some(_) if !force => {
            return Err(reject(
                StatusCode::CONFLICT,
                &format!(
                    "scenario '{}' already exists (pass force to replace it)",
                    b.name
                ),
            ));
        }
        None if c.scenarios.names().len() >= MAX_SCENARIOS => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!("at most {MAX_SCENARIOS} scenarios"),
            ));
        }
        _ => {}
    }
    let from = b.from.unwrap_or(SaveFrom::Live);
    // A file has no ttl: the leased live fields it writes are permanent once loaded (L5).
    // `effective` includes the live layer, so it carries the same leased fields.
    let leased = if matches!(from, SaveFrom::Live | SaveFrom::Effective) {
        chaos_lease::leased_fields(&c.chaos_leases, c.stack.live())
    } else {
        0
    };
    let warnings: Vec<String> = match leased {
        0 => vec![],
        1 => vec!["1 field has a ttl; the file has none".into()],
        n => vec![format!("{n} fields have a ttl; the file has none")],
    };
    let SaveSource {
        chaos: entries,
        pins,
        description,
    } = save_source(&st, &c, from)?;
    if entries.is_empty() && pins.is_empty() {
        return Err(reject(StatusCode::BAD_REQUEST, "nothing to save"));
    }
    let yaml = scenario_yaml(description.as_deref(), &entries, &pins);
    // The memory copy comes from the same parse a reload would do, so disk and memory agree.
    let cfg = parse_scenario(&yaml).map_err(|e| reject(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let (scenario, _) = resolve_scenario(&b.name, &cfg, &st.topo, &st.endpoints, &|ep| {
        st.empty_ok_for(ep)
    })
    .map_err(|e| reject(StatusCode::BAD_REQUEST, &place(&e.path, &e.message)))?;
    // ponytail: blocking file IO under the control mutex; a save is rare and tiny. Move the write
    // out (stage, then publish) if a slow disk ever stalls the control plane.
    let path = scenarios::write(&st.scenarios_dir, &b.name, &yaml, force).map_err(|e| match e {
        WriteError::Exists => reject(
            StatusCode::CONFLICT,
            &format!(
                "scenarios/{}.yaml already exists (pass force to replace it)",
                b.name
            ),
        ),
        WriteError::Other(m) => reject(StatusCode::INTERNAL_SERVER_ERROR, &m),
    })?;
    let was_active = c.stack.active().contains(&b.name);
    let snap = commit(&st, &mut c, |c| {
        c.scenarios
            .set_file(scenario)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
        if b.clear_live.unwrap_or(false) {
            c.stack.live_mut().clear();
            c.stack.live_pins_mut().clear();
        }
        Ok(())
    })?;
    if was_active {
        emit(&st, "update", Some(&b.name), &snap);
    }
    Ok(Json(
        json!({ "name": b.name, "path": path.display().to_string(), "warnings": warnings }),
    ))
}

/// **Private.** The data of a `scenario` SSE message: `{ action, name, active }`, plus
/// `chaos_key` only when set (a live chaos expiry).
fn scenario_data(
    action: &str,
    name: Option<&str>,
    chaos_key: Option<&Value>,
    active: &[String],
) -> Value {
    let mut v = json!({ "action": action, "name": name, "active": active });
    if let Some(k) = chaos_key {
        v["chaos_key"] = k.clone();
    }
    v
}

/// `GET /__kaanal/events` — the SSE stream: `request` messages for finished requests and
/// `scenario` messages (`{ action, name, active }`) for stack changes. Actions: `activate`,
/// `deactivate`, `set` (`PUT /scenarios/active`), `clear` (`DELETE /scenarios`), `reset`,
/// `update` (an active scenario's definition changed) and `expire` (a `ttl` ran out). A live
/// chaos expiry is an `expire` with `name: null` and `chaos_key`, the key as an object with the
/// parts it has (`{"node":"db"}`, `{"endpoint":"GET /x"}` or both); only it carries `chaos_key`.
///
/// When a shutdown is requested the stream sends one last message, `shutdown` with
/// `{ "timeout_ms": N }` (the time left), and ends, so an open stream never holds the drain.
/// The events of requests that are cut at the deadline therefore do not reach followers.
/// With a tiny budget the connection can be cut before that message is written (the same race as
/// the `202` of `shutdown`), so a follower must treat a plain end of stream as a shutdown too.
// limitation: no cap on concurrent event streams. Each holds one broadcast receiver, and only a
// caller with the 256-bit token can open one. Add a connection limit if the token ever leaves the local user.
pub async fn events(
    State(st): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(st.events.subscribe()).map(|msg| {
        Some(Ok(match msg {
            Ok(BusEvent::Request(ev)) => SseEvent::default()
                .event("request")
                .json_data(&ev)
                .unwrap_or_default(),
            Ok(BusEvent::Scenario {
                action,
                name,
                chaos_key,
                active,
            }) => SseEvent::default()
                .event("scenario")
                .json_data(scenario_data(
                    action,
                    name.as_deref(),
                    chaos_key.as_ref(),
                    &active,
                ))
                .unwrap_or_default(),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                SseEvent::default().event("lagged").data(n.to_string())
            }
        }))
    });
    // A shutdown request ends the stream (`None` stops `map_while`) after one last message.
    let stop = WatchStream::new(st.shutdown.watch())
        .filter(Option::is_some)
        .map(|_| None);
    // Stop is the only way the merged stream ends, so a `shutdown` message at its end is
    // always right; `remaining` is `Some` then.
    let last = st.shutdown.clone();
    let stream = stream
        .merge(stop)
        .map_while(|item| item)
        .chain(tokio_stream::iter(std::iter::once_with(move || {
            let ms = last
                .remaining()
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            Ok(SseEvent::default()
                .event("shutdown")
                .json_data(json!({ "timeout_ms": ms }))
                .unwrap_or_default())
        })));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// The page reads the token from the URL fragment, which browsers never send to a server.
/// `no-referrer` keeps even the page URL out of any outgoing request. The CSP allows only the
/// page's own inline script/style and same-origin calls, and `frame-ancestors 'none'` stops
/// another site from framing the page and tricking a click on a chaos button.
pub async fn ui() -> impl IntoResponse {
    (
        [
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                 connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
        ],
        Html(include_str!("ui.html")),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ChaosPatch, activate, clear_node, ct_eq, expire_due, host_allowed, lease_task, lock,
        next_deadline, query_token, scenario_data, set_chaos,
    };
    use crate::chaos_lease::Unit;
    use crate::state::{AppState, BusEvent};
    use axum::Json;
    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use kaanal_core::{ChaosKey, ChaosLayer, Config};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    fn state() -> Arc<AppState> {
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  db: { type: database, flavor: postgres }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\nscenarios:\n  a: { chaos: [ { node: db, latency_ms: 1 } ] }\n  b: { chaos: [ { node: db, latency_ms: 2 } ] }\n";
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let st = AppState::build(&Config::from_yaml(yaml).unwrap(), &dir, String::new(), 0)
            .map_err(|e| e.to_string())
            .unwrap();
        Arc::new(st)
    }

    async fn lease(st: &Arc<AppState>, name: &str, ttl: &str) {
        let body = Bytes::from(format!(r#"{{"ttl":"{ttl}"}}"#));
        let _ = activate(State(Arc::clone(st)), Path(name.into()), body)
            .await
            .unwrap();
    }

    async fn settle() {
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    fn expiries(rx: &mut tokio::sync::broadcast::Receiver<BusEvent>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let BusEvent::Scenario {
                action: "expire",
                name: Some(n),
                ..
            } = ev
            {
                out.push(n);
            }
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn lease_task_expires_on_the_paused_clock() {
        let st = state();
        let mut rx = st.events.subscribe();
        tokio::spawn(lease_task(Arc::clone(&st)));
        lease(&st, "a", "5s").await;
        tokio::time::advance(Duration::from_secs(4)).await;
        settle().await;
        assert_eq!(st.snapshot().stack.active(), ["a"]);
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
        assert_eq!(expiries(&mut rx), ["a"]);
    }

    #[tokio::test(start_paused = true)]
    async fn lease_survives_a_poisoned_lock() {
        let st = state();
        let s2 = Arc::clone(&st);
        let _ = std::thread::spawn(move || {
            let _g = s2.control.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(st.control.is_poisoned());
        tokio::spawn(lease_task(Arc::clone(&st)));
        lease(&st, "a", "1s").await;
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
        assert!(lock(&st).leases.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn two_leases_expire_in_order_with_one_event_each() {
        let st = state();
        let mut rx = st.events.subscribe();
        tokio::spawn(lease_task(Arc::clone(&st)));
        lease(&st, "b", "2s").await;
        lease(&st, "a", "1s").await;
        tokio::time::advance(Duration::from_millis(1500)).await;
        settle().await;
        assert_eq!(st.snapshot().stack.active(), ["b"]);
        assert_eq!(expiries(&mut rx), ["a"]);
        tokio::time::advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
        assert_eq!(expiries(&mut rx), ["b"]);
        assert_eq!(expire_due(&st), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn two_leases_due_in_one_call_expire_in_order() {
        let st = state();
        let mut rx = st.events.subscribe();
        lease(&st, "b", "1s").await;
        lease(&st, "a", "1s").await;
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(expire_due(&st), 2);
        assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
        assert_eq!(expiries(&mut rx), ["a", "b"]);
    }

    async fn put(st: &Arc<AppState>, node: &str, body: serde_json::Value) {
        let p: ChaosPatch = serde_json::from_value(body).unwrap();
        let _ = set_chaos(State(Arc::clone(st)), Path(node.into()), Ok(Json(p)))
            .await
            .unwrap();
    }

    fn node(st: &AppState, name: &str) -> ChaosKey {
        ChaosKey::Node(st.topo.id(name).unwrap())
    }

    /// The live entry of `db` in Control and in the published snapshot; they must agree.
    fn db_live(st: &AppState) -> Option<ChaosLayer> {
        let k = node(st, "db");
        let snap = st.snapshot().stack.live().layer(&k).cloned();
        assert_eq!(lock(st).stack.live().layer(&k).cloned(), snap);
        snap
    }

    fn chaos_expiries(
        rx: &mut tokio::sync::broadcast::Receiver<BusEvent>,
    ) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let BusEvent::Scenario {
                action: "expire",
                name,
                chaos_key: Some(k),
                ..
            } = ev
            {
                assert_eq!(name, None);
                out.push(k);
            }
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn chaos_ttl_expires_and_clears_only_its_unit() {
        let st = state();
        tokio::spawn(lease_task(Arc::clone(&st)));
        put(&st, "db", json!({ "latency_ms": 200 })).await;
        put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
        tokio::time::advance(Duration::from_secs(29)).await;
        settle().await;
        assert_eq!(db_live(&st).unwrap().down, Some(true));
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        let l = db_live(&st).unwrap();
        assert_eq!(l.down, None);
        assert_eq!(l.latency_add, Some(Duration::from_millis(200)));
        let snap = st.snapshot();
        let r = snap
            .chaos
            .visit(st.topo.id("db").unwrap(), "GET /x")
            .unwrap();
        assert!(!r.down);
        assert_eq!(r.latency_add, Duration::from_millis(200));
        assert_eq!(snap.chaos_leases.len(), 0);
        assert_eq!(next_deadline(&lock(&st)), None);
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_removes_an_entry_left_empty() {
        let st = state();
        put(
            &st,
            "db",
            json!({ "down": true, "mode": "override", "ttl": "1s" }),
        )
        .await;
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(expire_due(&st), 1);
        assert_eq!(db_live(&st), None);
        assert!(st.snapshot().chaos.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_24h_scenario_lease_does_not_delay_a_1s_chaos_lease() {
        let st = state();
        let mut rx = st.events.subscribe();
        tokio::spawn(lease_task(Arc::clone(&st)));
        lease(&st, "a", "24h").await;
        settle().await; // the task now sleeps toward the 24 h deadline
        put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(1001)).await;
        settle().await;
        assert_eq!(db_live(&st).and_then(|l| l.down), None);
        assert_eq!(chaos_expiries(&mut rx), [json!({ "node": "db" })]);
        assert_eq!(st.snapshot().stack.active(), ["a"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_reput_just_before_the_deadline_wins() {
        let st = state();
        tokio::spawn(lease_task(Arc::clone(&st)));
        put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
        tokio::time::advance(Duration::from_secs(29)).await;
        settle().await;
        put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(db_live(&st).unwrap().down, Some(true));
        // Compared under the lock: a due wake-up finds the replaced deadline and does nothing.
        assert_eq!(expire_due(&st), 0);
        tokio::time::advance(Duration::from_secs(27)).await;
        settle().await;
        assert_eq!(db_live(&st).unwrap().down, Some(true));
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(db_live(&st), None);
    }

    #[tokio::test(start_paused = true)]
    async fn plain_put_just_before_the_deadline_makes_it_permanent() {
        let st = state();
        let mut rx = st.events.subscribe();
        tokio::spawn(lease_task(Arc::clone(&st)));
        put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
        tokio::time::advance(Duration::from_secs(29)).await;
        settle().await;
        put(&st, "db", json!({ "down": true })).await;
        tokio::time::advance(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(db_live(&st).unwrap().down, Some(true));
        assert_eq!(next_deadline(&lock(&st)), None);
        assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_after_the_entry_was_deleted_emits_nothing_and_does_not_spin() {
        let st = state();
        let mut rx = st.events.subscribe();
        tokio::spawn(lease_task(Arc::clone(&st)));
        put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
        let _ = clear_node(State(Arc::clone(&st)), Path("db".into()))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(next_deadline(&lock(&st)), None);
        assert_eq!(expire_due(&st), 0);
        assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
        // Even a stale lease (no entry behind it) is dropped, silently.
        lock(&st).chaos_leases.insert(
            (node(&st, "db"), Unit::Down),
            tokio::time::Instant::now() - Duration::from_secs(1),
        );
        assert_eq!(expire_due(&st), 0);
        assert_eq!(next_deadline(&lock(&st)), None);
        assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
    }

    #[tokio::test(start_paused = true)]
    async fn expire_event_names_the_chaos_key() {
        let st = state();
        let mut rx = st.events.subscribe();
        put(&st, "db", json!({ "latency_ms": 5, "ttl": "1s" })).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(expire_due(&st), 1);
        let Ok(BusEvent::Scenario {
            action,
            name,
            chaos_key,
            active,
        }) = rx.try_recv()
        else {
            panic!("no scenario event");
        };
        assert_eq!((action, name.as_deref()), ("expire", None));
        assert_eq!(chaos_key, Some(json!({ "node": "db" })));
        assert_eq!(
            scenario_data(action, name.as_deref(), chaos_key.as_ref(), &active),
            json!({ "action": "expire", "name": null, "chaos_key": { "node": "db" }, "active": [] })
        );
        // An endpoint and a node@endpoint key name their parts the way `PUT /chaos` does.
        // (Stored directly: the test topology has no routes for a keyed PUT.)
        let id = st.topo.id("db").unwrap();
        let past = tokio::time::Instant::now() - Duration::from_secs(1);
        {
            let mut c = lock(&st);
            for (i, k) in [
                ChaosKey::Endpoint("GET /x".into()),
                ChaosKey::NodeAt(id, "GET /x".into()),
            ]
            .into_iter()
            .enumerate()
            {
                let down = ChaosLayer {
                    down: Some(true),
                    ..ChaosLayer::default()
                };
                c.stack.live_mut().set(k.clone(), down);
                let d = past + Duration::from_millis(u64::try_from(i).unwrap());
                c.chaos_leases.insert((k, Unit::Down), d);
            }
        }
        assert_eq!(expire_due(&st), 2);
        assert_eq!(
            chaos_expiries(&mut rx),
            [
                json!({ "endpoint": "GET /x" }),
                json!({ "node": "db", "endpoint": "GET /x" })
            ]
        );
        // A scenario event carries no `chaos_key` at all.
        assert_eq!(
            scenario_data("activate", Some("a"), None, &["a".into()]),
            json!({ "action": "activate", "name": "a", "active": ["a"] })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn two_units_of_one_key_expiring_together_publish_once_and_emit_once() {
        let st = state();
        let mut rx = st.events.subscribe();
        put(
            &st,
            "db",
            json!({ "latency_ms": 5, "down": true, "ttl": "1s" }),
        )
        .await;
        put(&st, "app", json!({ "down": true, "ttl": "1s" })).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        // One commit (one publish) per key.
        assert_eq!(expire_due(&st), 2);
        let mut keys = chaos_expiries(&mut rx);
        keys.sort_by_key(ToString::to_string);
        assert_eq!(keys, [json!({ "node": "app" }), json!({ "node": "db" })]);
        assert_eq!(db_live(&st), None);
        assert_eq!(next_deadline(&lock(&st)), None);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_expiry_drops_the_lease_and_keeps_the_chaos() {
        let st = state();
        let mut rx = st.events.subscribe();
        put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
        // Inject a failure: an out-of-range entry stored without a publish makes every
        // recompile fail, as a broken invariant would.
        lock(&st).stack.live_mut().set(
            node(&st, "app"),
            ChaosLayer {
                error_rate: Some(2.0),
                ..ChaosLayer::default()
            },
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(expire_due(&st), 0);
        assert_eq!(db_live(&st).unwrap().down, Some(true));
        assert_eq!(
            next_deadline(&lock(&st)),
            None,
            "a kept past-due lease would spin"
        );
        assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
    }

    #[test]
    fn hosts() {
        assert!(host_allowed("localhost:8080", 8080));
        assert!(host_allowed("127.0.0.1:8080", 8080));
        assert!(host_allowed("[::1]:8080", 8080));
        assert!(host_allowed("localhost", 8080));
        assert!(host_allowed("LocalHost:8080", 8080));
        assert!(!host_allowed("localhost:9999", 8080));
        assert!(!host_allowed("evil.example", 8080));
        assert!(!host_allowed("localhost.evil.example:8080", 8080));
        assert!(!host_allowed("evil.example:8080@localhost", 8080));
        assert!(!host_allowed("[::1", 8080));
        assert!(!host_allowed("", 8080));
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn token_comes_only_from_the_token_parameter() {
        assert_eq!(query_token(Some("token=abc")), Some("abc"));
        assert_eq!(query_token(Some("a=1&token=abc&b=2")), Some("abc"));
        assert_eq!(query_token(Some("xtoken=abc")), None);
        assert_eq!(query_token(Some("token")), None);
        assert_eq!(query_token(None), None);
    }
}
