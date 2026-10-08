//! The control API handlers and the security guard that stands in front of them.
//!
//! Every `/__kaanal/*` request passes [`guard`] before any handler or extractor
//! runs: Host must be loopback with the bound port, and every route except
//! `health`/`ui` needs the bearer token.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use kaanal_core::NodeChaos;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::{Stream, StreamExt};

use crate::state::AppState;

/// **Private.** Uniform JSON error shape returned by handlers: `(status, {"error": msg})`.
type Reject = (StatusCode, Json<Value>);

/// **Private.** Longest extra latency a chaos setting may add. A typo like 1e12 would otherwise hang every request.
const MAX_LATENCY_MS: u64 = 300_000;

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

/// **Private.** The public JSON shape of a node's chaos settings.
#[derive(Serialize)]
struct ChaosView {
    latency_ms: u64,
    error_rate: f64,
    error_status: u16,
    down: bool,
}

impl From<&NodeChaos> for ChaosView {
    fn from(c: &NodeChaos) -> Self {
        ChaosView {
            latency_ms: u64::try_from(c.latency_add.as_millis()).unwrap_or(u64::MAX),
            error_rate: c.error_rate,
            error_status: c.error_status,
            down: c.down,
        }
    }
}

/// The accepted body of `PUT /__kaanal/chaos/{node}`.
///
/// All fields are optional and merged into the node's existing settings;
/// unknown keys are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaosPatch {
    latency_ms: Option<u64>,
    error_rate: Option<f64>,
    error_status: Option<u16>,
    down: Option<bool>,
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
                "type": format!("{:?}", n.kind).to_lowercase(),
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

/// `GET /__kaanal/chaos` — all active chaos, keyed by node name.
pub async fn get_chaos(State(st): State<Arc<AppState>>) -> Json<Value> {
    let chaos = st.chaos.read().expect("chaos lock poisoned");
    let map: Map<String, Value> = chaos
        .iter()
        .map(|(id, c)| (st.topo.nodes[*id].name.clone(), json!(ChaosView::from(c))))
        .collect();
    Json(Value::Object(map))
}

/// `PUT /__kaanal/chaos/{node}` — merges a [`ChaosPatch`] into the node's settings
/// (omitted fields keep their values) and returns the merged state.
///
/// Validates ranges (`error_rate` 0–1, `error_status` 500–504, `latency_ms` ≤
/// [`MAX_LATENCY_MS`]); unknown node → 404, bad body → 4xx from the extractor.
///
/// # Errors
/// Returns [`Reject`] (400/404 with a JSON `error` field) on invalid input.
pub async fn set_chaos(
    State(st): State<Arc<AppState>>,
    Path(node): Path<String>,
    payload: Result<Json<ChaosPatch>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let id = st
        .topo
        .id(&node)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown node '{node}'")))?;
    let Json(p) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    if p.error_rate.is_some_and(|r| !(0.0..=1.0).contains(&r)) {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "error_rate must be between 0 and 1",
        ));
    }
    if p.error_status.is_some_and(|s| !(500..=504).contains(&s)) {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "error_status must be 500-504",
        ));
    }
    if p.latency_ms.is_some_and(|ms| ms > MAX_LATENCY_MS) {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            &format!("latency_ms must be at most {MAX_LATENCY_MS}"),
        ));
    }
    let mut chaos = st.chaos.write().expect("chaos lock poisoned");
    let c = chaos.entry(id).or_default();
    if let Some(ms) = p.latency_ms {
        c.latency_add = Duration::from_millis(ms);
    }
    if let Some(r) = p.error_rate {
        c.error_rate = r;
    }
    if let Some(s) = p.error_status {
        c.error_status = s;
    }
    if let Some(d) = p.down {
        c.down = d;
    }
    Ok(Json(json!(ChaosView::from(&*c))))
}

/// `DELETE /__kaanal/chaos/{node}` — removes one node's chaos (204).
///
/// # Errors
/// Returns [`Reject`] 404 for an unknown node name.
pub async fn clear_node(
    State(st): State<Arc<AppState>>,
    Path(node): Path<String>,
) -> Result<StatusCode, Reject> {
    let id = st
        .topo
        .id(&node)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown node '{node}'")))?;
    st.chaos.write().expect("chaos lock poisoned").remove(&id);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /__kaanal/chaos` — wipes all chaos (204).
pub async fn clear_all(State(st): State<Arc<AppState>>) -> StatusCode {
    st.chaos.write().expect("chaos lock poisoned").clear();
    StatusCode::NO_CONTENT
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
