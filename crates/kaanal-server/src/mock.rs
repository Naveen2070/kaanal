//! The mock request handler — the axum fallback that serves every non-control route.
//!
//! One request, end to end: CORS → matchit routing → chain execution (pure, via
//! `kaanal_core::execute`) → sleep the simulated total → static or generated body →
//! `X-Kaanal-Trace` header + an event on the broadcast bus.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, HopKind, Topology, execute, stable_hash,
};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use crate::state::{AppState, Body, Event};

/// Serves one mock request.
///
/// The engine runs synchronously in this task (microseconds of pure math); the
/// simulated latency is then awaited, so slow virtual paths never block a worker.
/// The `/__kaanal` namespace never reaches here (control routes win), but even a
/// hypothetical miss gets no CORS headers (rule R5.4).
pub async fn handle(State(st): State<Arc<AppState>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    // The control namespace never gets CORS (RULES R5.4), not even on a 404 or a preflight.
    let control = path == "/__kaanal" || path.starts_with("/__kaanal/");
    let cors = |r: Response| if control { r } else { with_cors(r) };

    if req.method() == Method::OPTIONS {
        return cors(StatusCode::NO_CONTENT.into_response());
    }
    let method = req.method().as_str().to_string();

    let Ok(matched) = st.paths.at(&path) else {
        return cors(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "no route", "path": path })),
            )
                .into_response(),
        );
    };
    // A HEAD with no spec entry was given its own copy of GET at build time (R6.8); hyper strips the body.
    let Some(route) = matched.value.iter().find(|r| r.method == method) else {
        let body = Json(json!({ "error": "method not allowed", "path": path }));
        return cors((StatusCode::METHOD_NOT_ALLOWED, body).into_response());
    };
    let params: Vec<(String, String)> = matched
        .params
        .iter()
        // Invalid escapes are kept as typed; invalid UTF-8 becomes U+FFFD.
        .map(|(k, v)| (k.to_string(), percent_decode_str(v).decode_utf8_lossy().into_owned()))
        .collect();

    let started = Instant::now();
    let n = route.counter.fetch_add(1, Ordering::Relaxed);
    let outcome = {
        let chaos = st.chaos.read().expect("chaos lock poisoned");
        let req = ChainRequest {
            entry: route.owner,
            op: route.op,
            endpoint_key: &route.key,
            n,
            seed: st.seed,
        };
        execute(&st.topo, &chaos, &req)
    };
    // limitation: on Windows the OS timer ticks about every 15 ms, so a sleep shorter than that
    // rounds up. Measured latency (the `ms` in events) can exceed the simulated sum shown in the trace.
    // Linux and macOS are exact to about 1 ms. Raise the timer resolution if sub-15 ms accuracy matters.
    tokio::time::sleep(outcome.total()).await;

    let name = |id: usize| st.topo.nodes[id].name.as_str();
    let trace = trace_header(&outcome, &st.topo);

    let (status, body, failed) = match outcome.result {
        ChainResult::Err { node, status } => (
            status,
            json!({ "error": "chaos", "node": name(node) }),
            Some(name(node).to_string()),
        ),
        ChainResult::Ok => {
            let body = match &route.body {
                Body::Static(v) => v.clone(),
                Body::Generated {
                    schema: Some(schema),
                } => {
                    let owner = route.owner.map_or("", name);
                    let mut v = kaanal_data::generate(
                        schema,
                        None,
                        // `seed:` is part of the key: same seed → same data; a new seed reshuffles it (NFR-10).
                        stable_hash(&[&st.seed.to_le_bytes(), owner.as_bytes(), path.as_bytes()]),
                    );
                    kaanal_data::apply_params(&mut v, &params);
                    v
                }
                Body::Generated { schema: None } => Value::Null,
            };
            (route.status, body, None)
        }
    };

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = if body.is_null() {
        code.into_response()
    } else {
        (code, Json(body)).into_response()
    };
    if let Ok(v) = HeaderValue::from_str(&trace) {
        resp.headers_mut().insert("x-kaanal-trace", v);
    }

    let nodes = touched_nodes(&outcome, failed.as_deref(), &st.topo);
    send_event(&st, method, path, status, started, nodes, failed);

    cors(resp)
}

/// **Private.** The `X-Kaanal-Trace` value: comma-separated hops with one decimal
/// (`1.0ms`, never a truncated `0ms`); a failure appends `node.error=STATUS`.
fn trace_header(outcome: &ChainOutcome, topo: &Topology) -> String {
    let name = |id: usize| topo.nodes[id].name.as_str();
    // One decimal: real hops are often under 1 ms and must not show as `0ms`.
    let fmt_ms = |d: std::time::Duration| format!("{:.1}ms", d.as_secs_f64() * 1000.0);
    let mut parts: Vec<String> = outcome
        .hops
        .iter()
        .map(|hop| match hop.kind {
            HopKind::Net => format!("net={}", fmt_ms(hop.dur)),
            HopKind::Proc => format!("{}.proc={}", name(hop.node), fmt_ms(hop.dur)),
            HopKind::Op(op) => format!("{}.{}={}", name(hop.node), op.name(), fmt_ms(hop.dur)),
        })
        .collect();
    // The walk stops at the first failing node, so say which one and why.
    if let ChainResult::Err { node, status } = &outcome.result {
        parts.push(format!("{}.error={status}", name(*node)));
    }
    parts.join(",")
}

/// **Private.** Ordered unique node names the request touched, plus the failing node.
fn touched_nodes(outcome: &ChainOutcome, failed: Option<&str>, topo: &Topology) -> Vec<String> {
    let mut nodes: Vec<String> = Vec::new();
    for hop in &outcome.hops {
        let n = topo.nodes[hop.node].name.as_str();
        if !nodes.iter().any(|x| x == n) {
            nodes.push(n.to_string());
        }
    }
    if let Some(f) = failed
        && !nodes.iter().any(|x| x == f)
    {
        nodes.push(f.to_string());
    }
    nodes
}

/// **Private.** Broadcasts the finished request on the event bus. No subscribers is
/// fine: send only errors when nobody listens.
fn send_event(
    st: &AppState,
    method: String,
    path: String,
    status: u16,
    started: Instant,
    nodes: Vec<String>,
    failed: Option<String>,
) {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    // Measured, not simulated: shows what a client really waited (timer granularity, load).
    // The trace header keeps the simulated per-hop times.
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _ = st.events.send(Event {
        ts_ms,
        method,
        path,
        status,
        ms,
        nodes,
        failed_node: failed,
    });
}

/// **Private.** Adds permissive CORS headers (mock routes only) and exposes
/// `x-kaanal-trace` to browsers.
fn with_cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("x-kaanal-trace"),
    );
    r
}
