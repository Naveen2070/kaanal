//! The mock request handler — the axum fallback that serves every non-control route.
//!
//! One request, end to end: CORS → matchit routing → the endpoint's compiled chain
//! (`kaanal_core::sample_plan`) → the replay engine actor (queues, pools, rate limits, real
//! time) → `override`, a `collection:` operation, a built-in responder (login, JWKS), a generated
//! body, or the `error_map` response → `X-Kaanal-Trace` (+ `Retry-After` on a 429) + an event on
//! the broadcast bus. A built-in login and a collection add read their body before the chain runs,
//! so a bad body still costs the full chain latency and a chain failure still wins (spec section 5). A
//! collection changes only when the route answers normally, and at most once (see [`answer`]).
//! A `reset` or `malformed` failure at a wire node (the gateway or the route's owner) is not
//! JSON: it is a body that breaks off or is cut short (B2a Z7); see [`wire_failure`].

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
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
use crate::generation::Generation;
use crate::log::{self, LogEntry};
use crate::pins;
use crate::record::{self, Answer, Facts, Run, TraceParts};
use crate::scope::{self, Scoped};
use crate::state::{AppState, BusEvent, Route};
use crate::wire::Cut;

mod flight;
mod paging;
#[cfg(test)]
mod payload_tests;
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
pub async fn handle(State(st): State<Arc<AppState>>, mut req: Request) -> Response {
    let path = req.uri().path().to_string();
    // The control namespace never gets CORS (RULES R5.4), not even on a 404 or a preflight.
    let control = path == "/__kaanal" || path.starts_with("/__kaanal/");
    let cors = |r: Response| if control { r } else { with_cors(r) };

    if let Some(early) = early_answer(req.method(), control, &path) {
        return early;
    }
    // Held to the end of the request: an Apply waits for it, and while an Apply drains, a new
    // request waits here, so it runs on the new system.
    let _in_flight = st.gate.enter().await;
    // One generation for the whole request: a swap mid-request never reaches it. A blank start has
    // no system yet, so there is nothing to answer.
    let Some(g) = st.generation() else {
        return cors(no_system());
    };
    // C11: a site file, when no spec route has this path. Before `Flight`, so it takes no id, no
    // counter and no log entry, and never meets the engine (chaos cannot hide the page).
    if let Some(resp) = site_page(&g, req.method(), &path, req.uri().query()).await {
        return resp;
    }
    // Preflights and site files take no id; everything below does, so ids count the requests that are logged.
    let mut flight = Flight::new(&st, &req, path);
    let (method, path) = (flight.method.as_str(), flight.path.as_str());

    let Ok(matched) = g.paths.at(path) else {
        let (ident, resp) = (flight.ident.take().expect("armed"), no_route(path));
        return cors(rejected(&st, ident, method, path, None, "unmatched", resp));
    };
    // A HEAD with no spec entry was given its own copy of GET at build time (R6.8); hyper strips the body.
    let Some(route) = matched.value.iter().find(|r| r.method == method) else {
        let ident = flight.ident.take().expect("armed");
        let resp = no_method(path);
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
    let params = decoded_params(&matched.params);
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
    let body_len = request_len(&g, route, &mut req).await;
    let body = read_body(route, &params, req).await;
    let sizes = sizes(
        (&st, &g),
        route,
        // an edit is only the body of an unscoped request
        (body_len, body.1.as_ref(), scope.is_none()),
        (method, path, &params, &query),
    );
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
    let (plan, pin) = plan_of(&g, route, n, chaos, pins, sizes);
    // limitation: the answer comes from the engine task, which wakes on the OS timer (about 15 ms
    // on Windows) but runs every hop at its scheduled time, so the trace stays exact.
    let Some(fin) = (match &g.engine {
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
        &g,
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

/// **Private.** The path parameters, percent-decoded: invalid escapes are kept as typed and invalid
/// UTF-8 becomes U+FFFD.
fn decoded_params(params: &matchit::Params<'_, '_>) -> Vec<(String, String)> {
    params
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                percent_decode_str(v).decode_utf8_lossy().into_owned(),
            )
        })
        .collect()
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

/// **Private.** What a request brings to its answer, read before the chain (spec section 5): a built-in
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

/// **Private.** The site file for `path`, when the config has a `site:` and no spec route has it.
async fn site_page(
    g: &Generation,
    method: &Method,
    path: &str,
    query: Option<&str>,
) -> Option<Response> {
    let site = g.site.as_ref()?;
    if g.paths.at(path).is_ok() {
        return None;
    }
    crate::site::serve(site, method, path, query).await
}

/// **Private.** The 405 for a path that has no route for this method.
fn no_method(path: &str) -> Response {
    let body = Json(json!({ "error": "method not allowed", "path": path }));
    (StatusCode::METHOD_NOT_ALLOWED, body).into_response()
}

/// **Private.** The answers that need no system: a preflight (204; CORS everywhere but the control
/// namespace) and an unknown control path (404, never CORS).
fn early_answer(method: &Method, control: bool, path: &str) -> Option<Response> {
    if *method == Method::OPTIONS {
        let r = StatusCode::NO_CONTENT.into_response();
        return Some(if control { r } else { with_cors(r) });
    }
    control.then(|| no_route(path))
}

/// **Private.** The 404 of a blank start before its first Apply.
fn no_system() -> Response {
    let body = Json(json!({ "error": "no system applied yet" }));
    (StatusCode::NOT_FOUND, body).into_response()
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
    g: &Generation,
    route: &Route,
    n: u64,
    chaos: &EffectiveChaos,
    pins: &EffectivePins,
    (req_bytes, resp_bytes): (u64, u64),
) -> (Arc<Plan>, Option<Outcome>) {
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
        // the time since the system started: chaos that begins mid-run and `detect` read it
        at: g.started.elapsed(),
        endpoint: ep,
        n,
        seed: g.chain_seed,
        pin_delay: pin.as_ref().map_or(Duration::ZERO, |o| o.delay),
        req_bytes,
        resp_bytes,
    };
    // `least-conn` and `p2c` stay open in the plan: the engine chooses by its live queues when the
    // call is sent, as in a simulation (step 8L; the mock used to settle them as for no load)
    (Arc::new(sample_plan(&g.topo, chaos, &req).0), pin)
}

/// What a route answers with no live edit: status, body and a file `override:`'s headers, for the
/// payload editor's starting text (and the `content_type` of a text body). `key` is a normalized route key; path parameters are filled
/// with `1`. `None` for an unknown route.
pub(crate) fn preview(g: &Generation, key: &str) -> Option<Preview> {
    let (method, tpl) = key.split_once(' ')?;
    let path: String = tpl
        .split('/')
        .map(|s| if s.starts_with('{') { "1" } else { s })
        .collect::<Vec<_>>()
        .join("/");
    let matched = g.paths.at(&path).ok()?;
    let route = matched.value.iter().find(|r| r.method == method)?;
    let params = decoded_params(&matched.params);
    let ask = Ask {
        method,
        path: &path,
        params: &params,
        query: &[],
    };
    let (status, body) = reply::success(g, route, &ask);
    let headers = route
        .endpoint
        .response
        .as_ref()
        .map(|o| o.headers.clone())
        .unwrap_or_default();
    let content_type = route
        .endpoint
        .response
        .as_ref()
        .and_then(|o| o.content_type.clone());
    let encoding = route.endpoint.response.as_ref().and_then(|o| o.encoding);
    Some((status, body, headers, content_type, encoding))
}

/// **Private.** What [`preview`] returns: status, body, headers, content type.
pub(crate) type Preview = (
    u16,
    Value,
    std::collections::BTreeMap<String, String>,
    Option<String>,
    Option<kaanal_core::config::BodyEncoding>,
);

/// **Private.** The request body size from `Content-Length` (`None` when absent or not a number).
fn content_length(headers: &axum::http::HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// **Private.** The largest request body counted by reading it through: the largest size a journey
/// may declare (1 GiB).
const MAX_COUNTED: u64 = 1 << 30;

/// **Private.** The size of the request body for the size costs (RULES 65 point 5): `Content-Length`
/// when it is there, else, for a route that does not read the body itself, the bytes the client
/// really sends (a chunked upload), counted by reading the body through and keeping none of it.
/// 0 when the topology has no size cost, so the body is not touched then.
async fn request_len(g: &Generation, route: &Route, req: &mut Request) -> u64 {
    if !g.topo.payload_costs() {
        return 0;
    }
    if let Some(n) = content_length(req.headers()) {
        return n;
    }
    let ep = &route.endpoint;
    let reads_itself = ep.collection.is_some() || ep.responder == Some(Responder::Login);
    if reads_itself {
        return 0;
    }
    count_body(std::mem::take(req.body_mut())).await
}

/// **Private.** The bytes of `body`, read through up to [`MAX_COUNTED`] and dropped as they come.
async fn count_body(body: Body) -> u64 {
    use tokio_stream::StreamExt;
    let mut n = 0u64;
    let mut chunks = body.into_data_stream();
    while let Some(Ok(chunk)) = chunks.next().await {
        n = n.saturating_add(chunk.len() as u64);
        if n >= MAX_COUNTED {
            return MAX_COUNTED;
        }
    }
    n
}

/// **Private.** `(method, path, params, query)` of a request.
type Parts<'a> = (
    &'a str,
    &'a str,
    &'a [(String, String)],
    &'a [(String, String)],
);

/// **Private.** The size of the body this route answers when nothing fails, for link and `per_kb`
/// costs, built before the chain runs: the serialized [`reply::success`] body (pure and seeded, so it
/// is the body the client will get); for a collection route the body its operation would answer on the
/// collection as it is now (a preview, which changes nothing, so an add that a later failure drops
/// leaves no trace); for the JWKS route its key set. A login's reply is not sized: it is a signed
/// token whose length depends on the account, and signing it here would double the work of a login.
/// `call` is the collection operation `read_body` built; `body_len` is the request's size, read
/// before the body was taken (it needs the headers).
fn sizes(
    (st, g): (&AppState, &Generation),
    route: &Route,
    (body_len, call, edits): (u64, Option<&Call>, bool),
    (method, path, params, query): Parts,
) -> (u64, u64) {
    // Only a topology with a payload cost looks at sizes at all.
    if !g.topo.payload_costs() {
        return (0, 0);
    }
    (
        body_len,
        resp_bytes(
            (st, g),
            route,
            call,
            &Ask {
                method,
                path,
                params,
                query,
            },
            edits,
        ),
    )
}

/// **Private.** The response half of [`sizes`].
fn resp_bytes(
    (st, g): (&AppState, &Generation),
    route: &Route,
    call: Option<&Call>,
    ask: &Ask,
    edits: bool,
) -> u64 {
    let ep = &route.endpoint;
    // hyper strips a HEAD body, so the client never receives those bytes
    if ask.method == "HEAD" {
        return 0;
    }
    let body = if let Some(call) = call {
        call.preview(&st.collections).1
    } else {
        match ep.responder.filter(|_| ep.response.is_none()) {
            Some(Responder::Jwks) => auth::respond(Responder::Jwks, g.auth.as_ref(), None).1,
            // a login's reply is not sized (see `sizes`)
            Some(_) => return 0,
            None if ep.collection.is_some() => return 0,
            // a live edit is the body an unscoped request will get (step 8A); a scoped one never
            // sees it, so it is sized by the body it does get
            None => {
                if let Some(edit) = edits
                    .then(|| st.payloads())
                    .as_deref()
                    .and_then(|t| t.get_for(ep))
                {
                    return edit.body.len() as u64;
                }
                reply::success(g, route, ask).1
            }
        }
    };
    if body.is_null() {
        0
    } else {
        serde_json::to_vec(&body).map_or(0, |b| b.len() as u64)
    }
}
