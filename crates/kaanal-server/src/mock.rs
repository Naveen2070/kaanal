//! The mock request handler — the axum fallback that serves every non-control route.
//!
//! One request, end to end: CORS → matchit routing → the endpoint's compiled chain
//! (`kaanal_core::sample_plan`) → the replay engine actor (queues, pools, rate limits, real
//! time) → `override`, a built-in responder (login, JWKS), a generated body, or the
//! `error_map` response → `X-Kaanal-Trace` (+ `Retry-After` on a 429) + an event on the
//! broadcast bus. A built-in login reads its body before the chain runs, so a
//! bad body still costs the full chain latency and a chain failure still wins (spec §5).

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use kaanal_core::{
    ChainRequest, ChainResult, FinishedHop, HopKind, Plan, PlanItem, Responder, Topology,
    resolve_error, sample_plan, stable_hash,
};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use crate::auth;
use crate::state::{AppState, Event, Route};

/// Serves one mock request.
///
/// The plan is sampled in this task (microseconds of pure math); the engine actor then runs
/// it in real time, so slow virtual paths never block a worker.
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
    let ep = &route.endpoint;
    debug_assert_eq!(route.key, ep.key, "a route answers with its own endpoint");
    // Spec §5: a built-in login reads (and checks) its body before the chain, with no sleep
    // yet; an `override` replaces the responder, so its body is never read.
    let login = if ep.responder == Some(Responder::Login) && ep.response.is_none() {
        Some(auth::read_login(req).await)
    } else {
        None
    };
    let n = route.counter.fetch_add(1, Ordering::Relaxed);
    let plan = {
        let chaos = st.chaos.read().expect("chaos lock poisoned");
        let (plan, _) = sample_plan(
            &st.topo,
            &chaos,
            &ChainRequest {
                endpoint: ep,
                n,
                seed: st.chain_seed,
            },
        );
        Arc::new(plan)
    };
    // The read lock is gone before the await: the engine task owns time from here.
    // limitation: the answer comes from the engine task, which wakes on the OS timer (about 15 ms
    // on Windows) but runs every hop at its scheduled time, so the trace stays exact.
    let Some(fin) = (match &st.engine {
        Some(e) => e.run(Arc::clone(&plan)).await,
        None => None,
    }) else {
        return cors(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "engine stopped" })),
            )
                .into_response(),
        );
    };

    let name = |id: usize| st.topo.nodes[id].name.as_str();
    let trace = trace_header(&plan, &fin.hops, &fin.result, &st.topo);
    let mut retry = None;
    let (status, body, failed) = match &fin.result {
        // E5: node, then status, then class; else the failure's own status and the chaos body.
        ChainResult::Err {
            node,
            status,
            retry_after,
        } => {
            let (status, body) = resolve_error(&ep.error_map, name(*node), *status);
            let body = body.unwrap_or_else(|| json!({ "error": "chaos", "node": name(*node) }));
            retry = (status == 429).then_some(*retry_after).flatten();
            (status, body, Some(name(*node).to_string()))
        }
        ChainResult::Ok => {
            if let Some(ov) = &ep.response {
                (ov.status, ov.body.clone(), None)
            } else if let Some(r) = ep.responder {
                // The responder owns its status codes; the route's 200 and schema do not apply.
                let (status, body) = auth::respond(r, st.auth.as_ref(), login);
                (status, body, None)
            } else {
                let owner = ep.owner.map_or("", name);
                (
                    route.status,
                    generated_body(&st, route, owner, &path, &params),
                    None,
                )
            }
        }
    };

    let mut resp = response(status, body, &trace, retry);
    if ep.responder == Some(Responder::Login) {
        no_store(&mut resp);
    }

    let nodes = touched_nodes(&fin.hops, failed.as_deref(), &st.topo);
    send_event(&st, method, path, status, started, nodes, failed);

    cors(resp)
}

/// **Private.** The reply: status and JSON body (none for `null`), the trace header and, for a
/// 429, `Retry-After` in whole seconds rounded up (at least 1).
fn response(status: u16, body: Value, trace: &str, retry: Option<Duration>) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = if body.is_null() {
        code.into_response()
    } else {
        (code, Json(body)).into_response()
    };
    if let Some(d) = retry {
        let secs = d
            .as_secs()
            .saturating_add(u64::from(d.subsec_nanos() > 0))
            .max(1);
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(secs));
    }
    if let Ok(v) = HeaderValue::from_str(trace) {
        resp.headers_mut().insert("x-kaanal-trace", v);
    }
    resp
}

/// **Private.** The spec-schema body of a successful route (`null` when it has no schema).
fn generated_body(
    st: &AppState,
    route: &Route,
    owner: &str,
    path: &str,
    params: &[(String, String)],
) -> Value {
    let Some(schema) = &route.schema else {
        return Value::Null;
    };
    // `seed:` (not the chain seed) keys the data: same seed → same data (NFR-10, D14).
    let seed = stable_hash(&[&st.seed.to_le_bytes(), owner.as_bytes(), path.as_bytes()]);
    let mut v = kaanal_data::generate(schema, None, seed);
    kaanal_data::apply_params(&mut v, params);
    v
}

/// **Private.** P11: tokens and credential errors are never cached, whatever answered.
fn no_store(r: &mut Response) {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

/// **Private.** Longest `X-Kaanal-Trace` value in bytes (Node's undici rejects headers over
/// about 16 KiB). The error part is always kept, so a long failing node name can pass it.
const TRACE_CAP: usize = 4096;

/// **Private.** One piece of the trace: a hop, or the bracket structure of a `parallel`.
enum Piece {
    Hop(String),
    Open,
    Bar,
    Close,
}

/// **Private.** The `X-Kaanal-Trace` value: visible hops in plan order with one decimal
/// (`1.0ms`, never a truncated `0ms`); a `parallel` block is `[a,b|c]` (branches split by
/// `|`); cache hops are `node.hit` / `node.miss`; `node.wait=T` comes before a hop that
/// queued; a failure appends `node.error=STATUS`. Over [`TRACE_CAP`] bytes it stops before a
/// hop, closes open brackets and adds `,...+N hops` (ASCII, D13), N being the pieces left out.
fn trace_header(
    plan: &Plan,
    hops: &[FinishedHop],
    result: &ChainResult,
    topo: &Topology,
) -> String {
    let name = |id: usize| topo.nodes[id].name.as_str();
    let fmt_ms = |d: Duration| format!("{:.1}ms", d.as_secs_f64() * 1000.0);
    let mut pieces = Vec::new();
    // (end of the fork's items, index of its first branch marker) per open fork.
    let mut forks: Vec<(usize, usize)> = Vec::new();
    let mut next = 0; // index into `hops`
    for (i, item) in plan.items.iter().enumerate() {
        while forks.last().is_some_and(|&(end, _)| end == i) {
            forks.pop();
            pieces.push(Piece::Close);
        }
        match item {
            PlanItem::Fork { size, .. } => {
                forks.push((i + 1 + size, i + 1));
                pieces.push(Piece::Open);
            }
            PlanItem::Branch { .. } => {
                if forks.last().is_some_and(|&(_, first)| first != i) {
                    pieces.push(Piece::Bar);
                }
            }
            PlanItem::Hop(_) if hops.get(next).is_some_and(|h| h.idx == i) => {
                let fh = &hops[next];
                let h = &fh.hop;
                next += 1;
                if !fh.wait.is_zero() {
                    pieces.push(Piece::Hop(format!(
                        "{}.wait={}",
                        name(h.node),
                        fmt_ms(fh.wait)
                    )));
                }
                let d = fmt_ms(h.dur);
                pieces.push(Piece::Hop(match h.kind {
                    HopKind::Net => format!("net={d}"),
                    HopKind::Proc => format!("{}.proc={d}", name(h.node)),
                    HopKind::Op(op) => format!("{}.{}={d}", name(h.node), op.name()),
                    HopKind::CacheHit => format!("{}.hit={d}", name(h.node)),
                    HopKind::CacheMiss => format!("{}.miss={d}", name(h.node)),
                }));
            }
            PlanItem::Hop(_) | PlanItem::Fail { .. } => {}
        }
    }
    pieces.extend(forks.iter().map(|_| Piece::Close));
    let pieces = without_empty_blocks(pieces);
    let error = match result {
        ChainResult::Err { node, status, .. } => format!("{}.error={status}", name(*node)),
        ChainResult::Ok => String::new(),
    };
    let tail = if error.is_empty() { 0 } else { error.len() + 1 };
    let mut out = render(&pieces, usize::MAX, 0);
    if out.len() + tail > TRACE_CAP {
        out = render(&pieces, TRACE_CAP, tail);
    }
    if !error.is_empty() {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&error);
    }
    out
}

/// **Private.** Drops every `[...]` block with no hop inside (a `parallel` that a sibling's
/// failure cancelled before it started would otherwise print as `[|]`).
fn without_empty_blocks(pieces: Vec<Piece>) -> Vec<Piece> {
    // Open positions with the hops seen inside each; a block's range is dropped when it has none.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut drop: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        match p {
            Piece::Open => open.push((i, 0)),
            Piece::Hop(_) => {
                if let Some(top) = open.last_mut() {
                    top.1 += 1;
                }
            }
            Piece::Close => {
                if let Some((start, hops)) = open.pop() {
                    if hops == 0 {
                        drop.push(start..i + 1);
                    } else if let Some(parent) = open.last_mut() {
                        parent.1 += hops;
                    }
                }
            }
            Piece::Bar => {}
        }
    }
    pieces
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !drop.iter().any(|r| r.contains(i)))
        .map(|(_, p)| p)
        .collect()
}

/// **Private.** Joins `pieces` (commas between elements, none after `[` or `|`). Stops
/// before the first hop that would not leave room, within `cap`, for the open brackets,
/// the `,...+N hops` suffix and `tail` more bytes.
fn render(pieces: &[Piece], cap: usize, tail: usize) -> String {
    let total = pieces.iter().filter(|p| matches!(p, Piece::Hop(_))).count();
    let suffix = ",...+".len() + total.to_string().len() + " hops".len();
    let (mut out, mut first, mut depth, mut shown) = (String::new(), true, 0usize, 0usize);
    for p in pieces {
        match p {
            Piece::Hop(h) => {
                let sep = usize::from(!first);
                if out.len() + sep + h.len() + depth + suffix + tail > cap {
                    out.extend(std::iter::repeat_n(']', depth));
                    if !out.is_empty() {
                        out.push(',');
                    }
                    let _ = write!(out, "...+{} hops", total - shown);
                    return out;
                }
                if !first {
                    out.push(',');
                }
                out.push_str(h);
                (first, shown) = (false, shown + 1);
            }
            Piece::Open => {
                if !first {
                    out.push(',');
                }
                out.push('[');
                (first, depth) = (true, depth + 1);
            }
            Piece::Bar => {
                out.push('|');
                first = true;
            }
            Piece::Close => {
                out.push(']');
                (first, depth) = (false, depth.saturating_sub(1));
            }
        }
    }
    out
}

/// **Private.** Ordered unique node names the request touched, plus the failing node.
fn touched_nodes(hops: &[FinishedHop], failed: Option<&str>, topo: &Topology) -> Vec<String> {
    let mut nodes: Vec<String> = Vec::new();
    for hop in hops {
        let n = topo.nodes[hop.hop.node].name.as_str();
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
        HeaderValue::from_static("x-kaanal-trace, retry-after"),
    );
    r
}
