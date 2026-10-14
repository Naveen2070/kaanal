//! The control API handlers and the security guard that stands in front of them.
//!
//! Every `/__kaanal/*` request passes [`guard`] before any handler or extractor
//! runs: Host must be loopback with the bound port, and every route except
//! `health`/`ui` needs the bearer token.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use kaanal_core::{ChaosKey, ChaosLayer, ErrorKind, merge};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::{Stream, StreamExt};

use crate::state::AppState;

/// **Private.** Uniform JSON error shape returned by handlers: `(status, {"error": msg})`.
type Reject = (StatusCode, Json<Value>);

// limitation: error messages echo the node name from the URL path (`unknown node '...'`) unsanitized.
// It is JSON-encoded, so the browser UI is safe, and the CLI replaces control characters before printing.
// Other clients that print it to a terminal must do the same.
fn reject(code: StatusCode, msg: &str) -> Reject {
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

/// **Private.** The public JSON shape of one chaos entry.
#[derive(Serialize)]
struct ChaosView {
    latency_ms: u64,
    error_rate: f64,
    error_kind: &'static str,
    error_status: u16,
    down: bool,
}

impl From<&ChaosLayer> for ChaosView {
    /// Unset fields show their defaults (what the layer does on its own).
    fn from(l: &ChaosLayer) -> Self {
        let c = merge(&[l]);
        ChaosView {
            latency_ms: u64::try_from(c.latency_add.as_millis()).unwrap_or(u64::MAX),
            error_rate: c.error_rate,
            error_kind: c.error_kind.as_str(),
            error_status: c.error_status,
            down: c.down,
        }
    }
}

/// The accepted body of `PUT /__kaanal/chaos/{node}`.
///
/// All fields are optional and merged into the existing settings;
/// unknown keys are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaosPatch {
    latency_ms: Option<u64>,
    error_rate: Option<f64>,
    error_kind: Option<String>,
    error_status: Option<u16>,
    down: Option<bool>,
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
        })
    }
}

/// The accepted body of `PUT /__kaanal/chaos`: a [`ChaosPatch`] plus where it applies.
/// (Spelled out rather than `flatten`ed so unknown keys are still rejected.) Keep its patch fields in
/// step with [`ChaosPatch`]: a new chaos field must be added to both.
///
/// A `node` + `endpoint` pair is not checked against the endpoint's chain (a node it never visits just
/// never fires), and a `HEAD` route is a different key from its `GET`.
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
}

/// The query of `DELETE /__kaanal/chaos`. A misspelt key must be a 400, never "no filter" (which wipes all).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearQuery {
    node: Option<String>,
    endpoint: Option<String>,
}

/// **Private.** Merges `patch` into the layer under `key`, validates the merged result (limits
/// live in core, R6.9) and stores it; a bad patch changes nothing. Returns the new view.
fn apply_patch(st: &AppState, key: ChaosKey, patch: &ChaosPatch) -> Result<Json<Value>, Reject> {
    let patch = patch.layer()?;
    let mut chaos = st.chaos.write().expect("chaos lock poisoned");
    let mut cur = chaos.layer(&key).cloned().unwrap_or_default();
    cur.apply(&patch);
    cur.validate()
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &e))?;
    let view = json!(ChaosView::from(&cur));
    chaos.set(key, cur);
    Ok(Json(view))
}

/// **Private.** `METHOD /path` with the method upper-cased, the key `compile` uses.
fn endpoint_key(e: &str) -> String {
    match e.split_once(' ') {
        Some((m, p)) => format!("{} {}", m.to_ascii_uppercase(), p),
        None => e.to_string(),
    }
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

/// `GET /__kaanal/chaos` — all active chaos:
/// `{ "nodes": {name: view}, "endpoints": {key: view}, "node_at": [{node, endpoint, ...view}] }`.
pub async fn get_chaos(State(st): State<Arc<AppState>>) -> Json<Value> {
    let chaos = st.chaos.read().expect("chaos lock poisoned");
    let (mut nodes, mut endpoints, mut node_at) = (Map::new(), Map::new(), Vec::new());
    for (k, l) in chaos.entries() {
        let view = json!(ChaosView::from(l));
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
    Json(json!({ "nodes": nodes, "endpoints": endpoints, "node_at": node_at }))
}

/// `PUT /__kaanal/chaos/{node}` — merges a [`ChaosPatch`] into the node's settings
/// (omitted fields keep their values) and returns the merged state.
///
/// The merged result is checked by [`ChaosLayer::validate`] (`error_rate` 0–1, `error_status`
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
    apply_patch(&st, ChaosKey::Node(id), &p)
}

/// `PUT /__kaanal/chaos` — like [`set_chaos`] but keyed by `node`, `endpoint` or both (the node
/// only when visited through that endpoint). At least one is required; an unknown node or
/// endpoint → 404; an endpoint with no entry node (a static route and no gateway) → 400.
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
            let key = endpoint_key(e);
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
    };
    apply_patch(&st, key, &patch)
}

/// `DELETE /__kaanal/chaos/{node}` — removes one node's chaos (204).
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name.
pub async fn clear_node(
    State(st): State<Arc<AppState>>,
    Path(node): Path<String>,
) -> Result<StatusCode, Reject> {
    let id = node_id(&st, &node)?;
    st.chaos
        .write()
        .expect("chaos lock poisoned")
        .remove(&ChaosKey::Node(id));
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /__kaanal/chaos?node=&endpoint=` — removes exactly the entry under that key (204,
/// also when there was none); with no query, wipes all chaos.
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name.
pub async fn clear_keyed(
    State(st): State<Arc<AppState>>,
    q: Result<Query<ClearQuery>, QueryRejection>,
) -> Result<StatusCode, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let node = q.node.as_deref().map(|n| node_id(&st, n)).transpose()?;
    let key = match (node, q.endpoint.as_deref().map(endpoint_key)) {
        (Some(n), Some(e)) => ChaosKey::NodeAt(n, e),
        (Some(n), None) => ChaosKey::Node(n),
        (None, Some(e)) => ChaosKey::Endpoint(e),
        (None, None) => {
            st.chaos.write().expect("chaos lock poisoned").clear();
            return Ok(StatusCode::NO_CONTENT);
        }
    };
    st.chaos.write().expect("chaos lock poisoned").remove(&key);
    Ok(StatusCode::NO_CONTENT)
}
// limitation: no cap on concurrent event streams. Each holds one broadcast receiver, and only a
// caller with the 256-bit token can open one. Add a connection limit if the token ever leaves the local user.
pub async fn events(
    State(st): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(st.events.subscribe()).map(|msg| {
        Ok(match msg {
            Ok(ev) => SseEvent::default()
                .event("request")
                .json_data(&ev)
                .unwrap_or_default(),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                SseEvent::default().event("lagged").data(n.to_string())
            }
        })
    });
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
    use super::{ct_eq, host_allowed, query_token};

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
