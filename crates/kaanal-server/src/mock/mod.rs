//! The mock request handler — the axum fallback that serves every non-control route.
//!
//! One request, end to end: CORS → matchit routing → the endpoint's compiled chain
//! (`kaanal_core::sample_plan`) → the replay engine actor (queues, pools, rate limits, real
//! time) → `override`, a `collection:` operation, a built-in responder (login, JWKS), a generated
//! body, or the `error_map` response → `X-Kaanal-Trace` (+ `Retry-After` on a 429) + an event on
//! the broadcast bus. A built-in login and a collection add read their body before the chain runs,
//! so a bad body still costs the full chain latency and a chain failure still wins (spec §5). A
//! collection changes only when the route answers normally, and at most once (see [`answer`]).
//! A `reset` or `malformed` failure at a wire node (the gateway or the route's owner) is not
//! JSON: it is a body that breaks off or is cut short (B2a Z7); see [`wire_failure`].

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use kaanal_core::{
    ChainRequest, ChainResult, CollectionOp, EffectiveChaos, EffectivePins, Endpoint, FailKind,
    Finished, FinishedHop, Outcome, PinState, Plan, Responder, Topology, resolve, resolve_error,
    sample_plan,
};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use crate::auth;
use crate::collections::{self, Call};
use crate::log::{self, LogEntry};
use crate::pins;
use crate::record::{self, Answer, Facts, Run, TraceParts};
use crate::scope::{self, Scoped};
use crate::state::{AppState, BusEvent, Route};
use crate::wire::Cut;

mod flight;
mod paging;
mod reply;
#[cfg(test)]
mod tests;

use flight::{Flight, Served, engine_stopped, finish, no_route, rejected, response};
use reply::{Ask, Reply, chaos_reply, no_store, pinned_reply, touched_nodes, with_cors};

/// Serves one mock request.
///
/// The plan is sampled in this task (microseconds of pure math); the engine actor then runs
/// it in real time, so slow virtual paths never block a worker.
/// The `/__kaanal` namespace never reaches here (control routes win), but even a
/// hypothetical miss gets no CORS headers (rule R5.4) and no request id.
///
/// Every other response, rejected ones included, carries `X-Request-Id` and leaves exactly one
/// log entry and one event: [`rejected`] covers the paths that never ran a chain, and a client
/// that goes away mid-request leaves a `client-gone` entry (see [`Flight`]). Only 404, 405 and
/// a bad scope touch nothing else: `engine-stopped` still counts the route and samples its
/// plan (that order predates the log).
pub async fn handle(State(st): State<Arc<AppState>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    // The control namespace never gets CORS (RULES R5.4), not even on a 404 or a preflight.
    let control = path == "/__kaanal" || path.starts_with("/__kaanal/");
    let cors = |r: Response| if control { r } else { with_cors(r) };

    if req.method() == Method::OPTIONS {
        return cors(StatusCode::NO_CONTENT.into_response());
    }
    if control {
        return no_route(&path);
    }
    // C11: a site file, when no spec route has this path. Before `Flight`, so it takes no id, no
    // counter and no log entry, and never meets the engine (chaos cannot hide the page).
    if let Some(site) = &st.site
        && st.paths.at(&path).is_err()
        && let Some(resp) = crate::site::serve(site, req.method(), &path).await
    {
        return resp;
    }
    // Preflights and site files take no id; everything below does, so ids count the requests that are logged.
    let mut flight = Flight::new(&st, &req, path);
    let (method, path) = (flight.method.as_str(), flight.path.as_str());

    let Ok(matched) = st.paths.at(path) else {
        let (ident, resp) = (flight.ident.take().expect("armed"), no_route(path));
        return cors(rejected(&st, ident, method, path, None, "unmatched", resp));
    };
    // A HEAD with no spec entry was given its own copy of GET at build time (R6.8); hyper strips the body.
    let Some(route) = matched.value.iter().find(|r| r.method == method) else {
        let body = Json(json!({ "error": "method not allowed", "path": path }));
        let resp = (StatusCode::METHOD_NOT_ALLOWED, body).into_response();
        let ident = flight.ident.take().expect("armed");
        return cors(rejected(&st, ident, method, path, None, "unmatched", resp));
    };
    flight.endpoint = Some(Arc::clone(&route.endpoint));
    // Before the login body read and the counter, so a bad scope costs no draw (B2b-3).
    let scope = match scope_of(&st, req.headers()) {
        Ok(s) => s,
        Err(detail) => {
            let (ident, resp) = (flight.ident.take().expect("armed"), bad_scope(&detail));
            let key = Some(route.key.as_str());
            return cors(rejected(&st, ident, method, path, key, "bad-scope", resp));
        }
    };
    if let (Some(i), Some(s)) = (flight.ident.as_mut(), &scope) {
        i.scope = Some(s.0.clone());
    }
    let params: Vec<(String, String)> = matched
        .params
        .iter()
        // Invalid escapes are kept as typed; invalid UTF-8 becomes U+FFFD.
        .map(|(k, v)| (k.to_string(), percent_decode_str(v).decode_utf8_lossy().into_owned()))
        .collect();
    // Read before `read_body` takes the request; only paging reads it (an unpaged route skips
    // the parse), the link keeps the raw path.
    let query = route
        .paging
        .as_ref()
        .map_or_else(Vec::new, |_| query_pairs(req.uri()));

    debug_assert_eq!(
        route.key, route.endpoint.key,
        "a route answers with its own endpoint"
    );
    let body = read_body(route, &params, req).await;
    let n = route.counter.fetch_add(1, Ordering::Relaxed);
    // S10: the scoped stack, or else one snapshot of the live one (the `Arc` is cloned once here
    // and kept until the entry is built, so a swap mid-request never reaches this request). A
    // scoped request never sees the live pins.
    let snap;
    let (chaos, pins) = if let Some(s) = &scope {
        (&s.1.chaos, &s.1.pins)
    } else {
        snap = st.snapshot();
        (&snap.chaos, &snap.pins)
    };
    let (plan, pin) = plan_of(&st, route, n, chaos, pins);
    let plan = Arc::new(plan);
    // limitation: the answer comes from the engine task, which wakes on the OS timer (about 15 ms
    // on Windows) but runs every hop at its scheduled time, so the trace stays exact.
    let Some(fin) = (match &st.engine {
        Some(e) => e.run(Arc::clone(&plan)).await,
        None => None,
    }) else {
        let (ident, resp) = (flight.ident.take().expect("armed"), engine_stopped());
        let key = Some(route.key.as_str());
        return cors(rejected(
            &st,
            ident,
            method,
            path,
            key,
            "engine-stopped",
            resp,
        ));
    };
    let ident = flight.ident.take().expect("armed");
    let resp = finish(
        &st,
        body,
        Served {
            route,
            ident,
            method,
            path,
            params: &params,
            query: &query,
            scope: scope.as_ref().map(|s| s.0.as_str()),
            chaos,
            plan: &plan,
            pin: pin.as_ref(),
            fin: &fin,
        },
    );
    cors(resp)
}

/// **Private.** The decoded query pairs of `uri`, in order (`+` is a space; invalid escapes are
/// kept as typed and invalid UTF-8 becomes U+FFFD, like path parameters).
fn query_pairs(uri: &axum::http::Uri) -> Vec<(String, String)> {
    uri.query()
        .and_then(|q| serde_urlencoded::from_str(q).ok())
        .unwrap_or_default()
}

/// **Private.** What [`read_body`] read: a built-in login's credentials, or a collection operation.
/// Moved into the one place that answers, so a collection changes at most once per request.
type Read<'a> = (
    Option<Result<auth::Credentials, auth::Reject>>,
    Option<Call<'a>>,
);

/// **Private.** What a request brings to its answer, read before the chain (spec §5): a built-in
/// login's credentials, or its collection operation with an add's body (read with a hard cap). No
/// sleep yet, so a bad body still costs the full chain latency and a chain failure still wins. An
/// `override` replaces the responder, so its body is never read.
async fn read_body<'a>(route: &'a Route, params: &'a [(String, String)], req: Request) -> Read<'a> {
    let ep = &route.endpoint;
    if let (Some(name), Some(op)) = (ep.collection.as_deref(), collections::op_of(route)) {
        let item = if op == CollectionOp::Add {
            Some(collections::read_item(req).await)
        } else {
            None
        };
        // A by-id shape has exactly one path parameter; the name comes from the config alone.
        let id = params.first().map(|(_, v)| v.as_str());
        return (None, Some(Call { name, op, id, item }));
    }
    if ep.responder == Some(Responder::Login) && ep.response.is_none() {
        return (Some(auth::read_login(req).await), None);
    }
    (None, None)
}

/// **Private.** The request's scope: its key and compiled stack, `None` without the header.
/// The error is the detail of a 400.
fn scope_of(
    st: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<(String, Arc<Scoped>)>, String> {
    let Some(names) = scope::parse(headers)? else {
        return Ok(None);
    };
    let key = scope::key(&names);
    let scoped = st.scoped(&key, &names)?;
    Ok(Some((key, scoped)))
}

/// **Private.** The 400 for a bad `X-Kaanal-Scenario` header.
fn bad_scope(detail: &str) -> Response {
    let body = Json(json!({ "error": "bad scenario scope", "detail": detail }));
    (StatusCode::BAD_REQUEST, body).into_response()
}

/// **Private.** Samples the request's plan and settles its pin from one view (the `chaos` and
/// `pins` of the scoped stack or of the snapshot). A pin's delay goes into the plan; a
/// synthesized `HEAD` follows its `GET`'s pin.
fn plan_of(
    st: &AppState,
    route: &Route,
    n: u64,
    chaos: &EffectiveChaos,
    pins: &EffectivePins,
) -> (Plan, Option<Outcome>) {
    let ep = &route.endpoint;
    let pin = if pins.is_empty() {
        None
    } else if ep.synthesized {
        pins.get(&route.key.replacen("HEAD ", "GET ", 1))
    } else {
        pins.get(&route.key)
    };
    let pin = pin.map(|p| resolve(&p.entry, &ep.states));
    let req = ChainRequest {
        endpoint: ep,
        n,
        seed: st.chain_seed,
        pin_delay: pin.as_ref().map_or(Duration::ZERO, |o| o.delay),
    };
    (sample_plan(&st.topo, chaos, &req).0, pin)
}
