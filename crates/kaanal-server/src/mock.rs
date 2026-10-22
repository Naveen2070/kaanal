//! The mock request handler — the axum fallback that serves every non-control route.
//!
//! One request, end to end: CORS → matchit routing → the endpoint's compiled chain
//! (`kaanal_core::sample_plan`) → the replay engine actor (queues, pools, rate limits, real
//! time) → `override`, a built-in responder (login, JWKS), a generated body, or the
//! `error_map` response → `X-Kaanal-Trace` (+ `Retry-After` on a 429) + an event on the
//! broadcast bus. A built-in login reads its body before the chain runs, so a
//! bad body still costs the full chain latency and a chain failure still wins (spec §5).
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
    ChainRequest, ChainResult, EffectiveChaos, EffectivePins, Endpoint, FailKind, Finished,
    FinishedHop, Outcome, PinState, Plan, Responder, Topology, resolve, resolve_error, sample_plan,
};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use crate::auth;
use crate::log::{self, LogEntry};
use crate::pins;
use crate::record::{self, Answer, Facts, Run, TraceParts};
use crate::scope::{self, Scoped};
use crate::state::{AppState, BusEvent, Route};
use crate::wire::Cut;

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
    // Preflights take no id; everything below does, so ids count the requests that are logged.
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
        login,
        Served {
            route,
            ident,
            method,
            path,
            params: &params,
            scope: scope.as_ref().map(|s| s.0.as_str()),
            chaos,
            plan: &plan,
            pin: pin.as_ref(),
            fin: &fin,
        },
    );
    cors(resp)
}

/// **Private.** A mock request in flight. If the handler future is dropped (the client went away)
/// before it finished, the drop publishes a `client-gone` entry (status 499, no hops) and its
/// event, so every id ends in exactly one entry. Every normal exit takes `ident` first, which
/// disarms the guard; it never publishes twice.
struct Flight {
    st: Arc<AppState>,
    ident: Option<Ident>,
    method: String,
    path: String,
    endpoint: Option<Arc<Endpoint>>,
}

impl Flight {
    /// Takes the request's id and starts its clock.
    fn new(st: &Arc<AppState>, req: &Request, path: String) -> Self {
        let (seq, id) = st.ids.next();
        let client_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .and_then(log::client_id);
        Flight {
            st: Arc::clone(st),
            ident: Some(Ident {
                id,
                client_id,
                seq,
                started: Instant::now(),
                scope: None,
            }),
            method: req.method().as_str().to_string(),
            path,
            endpoint: None,
        }
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        // A panic unwinding through the handler is not a client abort.
        if let Some(mut ident) = self.ident.take().filter(|_| !std::thread::panicking()) {
            let key = self.endpoint.as_ref().map(|e| e.key.as_str());
            let scope = ident.scope.take();
            let facts = ident.facts(&self.method, &self.path, key, scope.as_deref());
            let entry = record::reject_entry(facts, 499, "client-gone");
            publish(&self.st, entry, Vec::new());
        }
    }
}

/// **Private.** What [`finish`] needs once the engine has answered.
struct Served<'a> {
    route: &'a Route,
    ident: Ident,
    method: &'a str,
    path: &'a str,
    params: &'a [(String, String)],
    scope: Option<&'a str>,
    chaos: &'a EffectiveChaos,
    plan: &'a Plan,
    pin: Option<&'a Outcome>,
    fin: &'a Finished,
}

/// **Private.** Builds the response of a request the chain served, then logs it and tells the bus.
fn finish(
    st: &AppState,
    login: Option<Result<auth::Credentials, auth::Reject>>,
    s: Served,
) -> Response {
    let Served {
        route,
        ident,
        method,
        path,
        params,
        scope,
        chaos,
        plan,
        pin,
        fin,
    } = s;
    let ep = &route.endpoint;
    let pinned = pin.map(|o| o.state.name());
    let TraceParts { full, header } =
        record::trace_parts(plan, &fin.hops, &fin.result, &st.topo, pinned);
    let ask = Ask {
        method,
        path,
        params,
    };
    // P2: a pin wins over whatever the chain did; the chain still ran (its hops and delay are real).
    let Reply {
        status,
        body,
        failed,
        retry,
        wire,
    } = match pin {
        Some(o) => pinned_reply(st, route, &ask, login, o),
        None => chaos_reply(st, route, &fin.result, &ask, login, &header),
    };

    let mut resp = wire.unwrap_or_else(|| response(status, body, &header, retry));
    if ep.responder == Some(Responder::Login) {
        no_store(&mut resp);
    }
    tag(&mut resp, &ident.id);
    if let Some(v) = scope.and_then(|k| HeaderValue::from_str(k).ok()) {
        resp.headers_mut().insert(scope::SCOPE_ECHO, v);
    }
    // The event lists what the chain touched, its failing node too, even when a pin hid the failure.
    let chain_failed = match &fin.result {
        ChainResult::Err { node, .. } => Some(st.topo.nodes[*node].name.as_str()),
        ChainResult::Ok => None,
    };
    let nodes = touched_nodes(&fin.hops, chain_failed, &st.topo);
    let facts = ident.facts(method, path, Some(route.key.as_str()), scope);
    let run = Run {
        plan,
        fin,
        chaos,
        topo: &st.topo,
        pin_delay: pin.is_some_and(|o| !o.delay.is_zero()),
    };
    let ans = Answer {
        status,
        pin: pinned,
        failed: failed.as_ref(),
    };
    publish(st, record::build_entry(facts, &ans, &run, full), nodes);
    resp
}

/// **Private.** The id and clock of one request, taken when it arrives.
struct Ident {
    id: String,
    client_id: Option<String>,
    seq: u64,
    started: Instant,
    /// The scope key once the header resolved, so a rejection or an abort after that keeps it.
    scope: Option<String>,
}

impl Ident {
    /// The request facts for its log entry. `ms` is measured now, not simulated: it shows what a
    /// client really waited (timer granularity, load). The trace keeps the simulated per-hop times.
    fn facts<'a>(
        self,
        method: &'a str,
        path: &'a str,
        endpoint: Option<&'a str>,
        scope: Option<&'a str>,
    ) -> Facts<'a> {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Facts {
            ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            id: self.id,
            client_id: self.client_id,
            seq: self.seq,
            ts_ms,
            method,
            path,
            endpoint,
            scope,
        }
    }
}

/// **Private.** Puts `X-Request-Id` on a response.
fn tag(resp: &mut Response, id: &str) {
    if let Ok(v) = HeaderValue::from_str(id) {
        resp.headers_mut().insert("x-request-id", v);
    }
}

/// **Private.** Logs a request refused before any chain ran and tags its response. These paths
/// touch neither the route counter nor a seeded draw.
fn rejected(
    st: &AppState,
    mut ident: Ident,
    method: &str,
    path: &str,
    endpoint: Option<&str>,
    why: &'static str,
    mut resp: Response,
) -> Response {
    tag(&mut resp, &ident.id);
    let status = resp.status().as_u16();
    let scope = ident.scope.take();
    let facts = ident.facts(method, path, endpoint, scope.as_deref());
    publish(st, record::reject_entry(facts, status, why), Vec::new());
    resp
}

/// **Private.** Records a finished request, then tells the bus. The ring comes first, so a
/// subscriber that sees the event can already find the entry. No subscribers is fine: send only
/// errors when nobody listens.
fn publish(st: &AppState, entry: LogEntry, nodes: Vec<String>) {
    let event = record::event_of(&entry, nodes);
    st.push_log(entry);
    let _ = st.events.send(BusEvent::Request(event));
}

/// **Private.** The 404 for a path nothing serves.
fn no_route(path: &str) -> Response {
    let body = Json(json!({ "error": "no route", "path": path }));
    (StatusCode::NOT_FOUND, body).into_response()
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

/// **Private.** The 500 for a replay engine that is gone.
fn engine_stopped() -> Response {
    let body = Json(json!({ "error": "engine stopped" }));
    (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
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

/// **Private.** What a request answers: status and body, the failing node, a `Retry-After` and,
/// for a wire failure, the broken response itself.
struct Reply {
    status: u16,
    body: Value,
    failed: Option<(String, FailKind)>,
    retry: Option<Duration>,
    wire: Option<Response>,
}

/// **Private.** The request line and path parameters a reply is built from.
struct Ask<'a> {
    method: &'a str,
    path: &'a str,
    params: &'a [(String, String)],
}

/// **Private.** The reply of a request with no pin: the chain's verdict decides.
fn chaos_reply(
    st: &AppState,
    route: &Route,
    result: &ChainResult,
    ask: &Ask,
    login: Option<Result<auth::Credentials, auth::Reject>>,
    trace: &str,
) -> Reply {
    let ep = &route.endpoint;
    let name = |id: usize| st.topo.nodes[id].name.as_str();
    match result {
        // E5 + B2a Z8: node, then kind, then status, then class; else the kind's default status and the chaos body.
        ChainResult::Err {
            node,
            status,
            kind,
            retry_after,
        } => {
            // A timeout at a wire node is not a wire failure: it stays a 504 that `error_map` may
            // remap (status or class keys, like nginx `error_page 504`); only reset and malformed break the wire.
            let at_wire = *kind != FailKind::Status
                && *kind != FailKind::Timeout
                && ep.is_wire_node(&st.topo, *node);
            let (mapped, body) = resolve_error(&ep.error_map, name(*node), *kind, *status);
            // An error_map entry by node name or by kind is the user's stated intent and beats the wire behaviour (Z7).
            // Same string lookup as `resolve_error`: a node named `reset` also counts as a kind key for other nodes.
            let overridden =
                ep.error_map.contains_key(name(*node)) || ep.error_map.contains_key(kind.as_str());
            let failed = Some((name(*node).to_string(), *kind));
            if at_wire && !overridden {
                let ok = answer(st, route, ask.path, ask.params, login);
                let (s, r) = wire_failure(*kind, ok, trace, ask.method);
                return Reply {
                    status: s,
                    body: Value::Null,
                    failed,
                    retry: None,
                    wire: Some(r),
                };
            }
            Reply {
                status: mapped,
                body: body.unwrap_or_else(|| default_error_body(name(*node), *kind)),
                failed,
                retry: (mapped == 429).then_some(*retry_after).flatten(),
                wire: None,
            }
        }
        ChainResult::Ok => {
            let (status, body) = answer(st, route, ask.path, ask.params, login);
            Reply {
                status,
                body,
                failed: None,
                retry: None,
                wire: None,
            }
        }
    }
}

/// **Private.** The reply of a pinned request (P2: the pin beats `down`, `error_map`, wire
/// failures, 429 and overflow; the chain's verdict is not read). Never a failed node.
fn pinned_reply(
    st: &AppState,
    route: &Route,
    ask: &Ask,
    login: Option<Result<auth::Credentials, auth::Reject>>,
    out: &Outcome,
) -> Reply {
    let (status, body, retry) = pinned_response(st, route, ask, login, out);
    Reply {
        status,
        body,
        failed: None,
        retry,
        wire: None,
    }
}

/// **Private.** What a pin answers. `loading`, `slow` and `success` answer as the route normally
/// does. `empty`: the derived body (`states.empty.body`, else the emptied `override:` body or
/// schema) with the path parameters applied, the pin's or `states:` status or the normal success
/// status; a 204 or 304 (the route's own or a pinned one) never has a body. `error`: the pin's status
/// (500 by default) with the `states.error.body` or a small default, plus `Retry-After: 1` for
/// 429 and 503. Never `error_map`, never a wire failure.
fn pinned_response(
    st: &AppState,
    route: &Route,
    ask: &Ask,
    login: Option<Result<auth::Credentials, auth::Reject>>,
    out: &Outcome,
) -> (u16, Value, Option<Duration>) {
    match out.state {
        PinState::Error => {
            let status = out.status.unwrap_or(500);
            let body = out
                .body
                .clone()
                .unwrap_or_else(|| json!({ "error": "pinned", "state": "error" }));
            let retry = matches!(status, 429 | 503).then_some(Duration::from_secs(1));
            (status, body, retry)
        }
        PinState::Empty => {
            let normal = route
                .endpoint
                .response
                .as_ref()
                .map_or(route.status, |o| o.status);
            let status = out.status.unwrap_or(normal);
            // A 204 or 304 never carries a body, whichever way the status was chosen.
            if matches!(status, 204 | 304) {
                return (status, Value::Null, None);
            }
            let ep = &route.endpoint;
            let owner = ep.owner.map_or("", |id| st.topo.nodes[id].name.as_str());
            let seed = pins::route_seed(st.seed, owner, ask.path);
            let mut body = pins::empty_form(ep, route.schema.as_ref(), seed).unwrap_or(Value::Null);
            // P9: a `states.empty.body` is returned verbatim; only a derived body gets the params.
            if ep.states.empty.as_ref().is_none_or(|o| o.body.is_none()) {
                kaanal_data::apply_params(&mut body, ask.params);
            }
            (status, body, None)
        }
        PinState::Loading | PinState::Slow | PinState::Success => {
            let (status, body) = answer(st, route, ask.path, ask.params, login);
            (status, body, None)
        }
    }
}

/// **Private.** The chaos body of a failure that answers as JSON. Kind `status` keeps the
/// original two-field shape; any other kind names itself.
fn default_error_body(node: &str, kind: FailKind) -> Value {
    if kind == FailKind::Status {
        json!({ "error": "chaos", "node": node })
    } else {
        json!({ "error": "chaos", "node": node, "kind": kind.as_str() })
    }
}

/// **Private.** What a route answers when nothing fails: the `override`, else the generated body.
fn success(st: &AppState, route: &Route, path: &str, params: &[(String, String)]) -> (u16, Value) {
    let ep = &route.endpoint;
    if let Some(ov) = &ep.response {
        return (ov.status, ov.body.clone());
    }
    let owner = ep.owner.map_or("", |id| st.topo.nodes[id].name.as_str());
    (route.status, generated_body(st, route, owner, path, params))
}

/// **Private.** What the route answers when nothing fails: a built-in responder (it owns its status
/// codes; the route's 200 and schema do not apply), else [`success`].
fn answer(
    st: &AppState,
    route: &Route,
    path: &str,
    params: &[(String, String)],
    login: Option<Result<auth::Credentials, auth::Reject>>,
) -> (u16, Value) {
    let ep = &route.endpoint;
    match ep.responder.filter(|_| ep.response.is_none()) {
        Some(r) => auth::respond(r, st.auth.as_ref(), login),
        None => success(st, route, path, params),
    }
}

/// **Private.** A `reset` or `malformed` failure at a wire node, as the route's success status
/// plus a broken body. Returns the status sent and the response.
///
/// - `malformed`: the serialized success body cut at about half (at a char boundary), with a
///   `Content-Length` that matches the cut, so the client gets valid HTTP with invalid JSON.
/// - `reset`: the first half as one chunk, then the stream errors while `Content-Length` still
///   promises the whole body, so the client sees the status and part of the body, then an abort.
///
/// `ok` is what the route would have answered (the real reply, built-in responders included).
/// A `HEAD`, a 204 and a 304 have no body to break: they get the plain success response, yet
/// the trace header still ends in `node.error=reset|malformed` and the event still names the failed node.
/// A `reset` promises at least one byte more than it sent, so the client always sees the abort.
// ponytail: a bare-number body ("12") can cut to valid JSON ("1"); schema bodies are objects or arrays in practice.
fn wire_failure(
    kind: FailKind,
    (status, body): (u16, Value),
    trace: &str,
    method: &str,
) -> (u16, Response) {
    let no_body = method == "HEAD" || matches!(status, 204 | 304);
    if no_body {
        return (status, response(status, body, trace, None));
    }
    // A route without a body still gets something to break.
    let full = if body.is_null() {
        "{}".to_string()
    } else {
        body.to_string()
    };
    let mut cut = full.len() / 2;
    while !full.is_char_boundary(cut) {
        cut -= 1;
    }
    let part = if cut == 0 { "{\"" } else { &full[..cut] };
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = if kind == FailKind::Reset {
        let mut r = Response::new(Body::from_stream(Cut::new(Bytes::copy_from_slice(
            part.as_bytes(),
        ))));
        r.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from(full.len().max(part.len() + 1)),
        );
        r
    } else {
        Response::new(Body::from(part.to_string()))
    };
    *resp.status_mut() = code;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Ok(v) = HeaderValue::from_str(trace) {
        resp.headers_mut().insert("x-kaanal-trace", v);
    }
    (status, resp)
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
    let seed = pins::route_seed(st.seed, owner, path);
    let opts = st.options_for_owner(route.endpoint.owner);
    let mut v = kaanal_data::generate(schema, None, seed, opts);
    kaanal_data::apply_params(&mut v, params);
    v
}

/// **Private.** P11: tokens and credential errors are never cached, whatever answered.
fn no_store(r: &mut Response) {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
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
        HeaderValue::from_static("x-kaanal-trace, x-kaanal-scope, x-request-id, retry-after"),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use kaanal_core::Config;
    use kaanal_core::replay::Engine;

    const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
  h: { type: external-api, proc: 1ms, timeout: 150ms }
  c: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 2ms }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: app, to: h, net: 1ms }
  - { from: app, to: c, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
  \"GET /users\": { chain: [ { cache: c } ] }
  \"POST /orders\": { chain: [ h ] }
scenarios:
  hang: { chaos: [ { node: h, error_rate: 1.0, error_kind: hang, latency_ms: 30 } ] }
  cachelag: { chaos: [ { node: c, latency_ms: 40 } ] }
  eplag: { chaos: [ { endpoint: \"GET /users/{id}\", latency_ms: 20 } ] }
  dbhang: { chaos: [ { node: db, error_rate: 1.0, error_kind: hang, latency_ms: 30 } ] }
  hreset:
    chaos: [ { node: db, error_rate: 1.0, error_kind: reset } ]
    pins: [ { endpoint: \"GET /users/{id}\", state: success } ]
  lag: { chaos: [ { node: db, latency_ms: 300 } ] }
  down: { chaos: [ { node: db, error_rate: 1.0, error_status: 503 } ] }
  hidden:
    chaos: [ { node: db, error_rate: 1.0, error_status: 503 } ]
    pins: [ { endpoint: \"GET /users/{id}\", state: success } ]
";

    type Req<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

    /// The server state of `YAML` with a live engine, driven by calling [`handle`] directly: the
    /// ring is read through `st.log`, which no route serves yet.
    fn app() -> Arc<AppState> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let cfg = Config::from_yaml(YAML).unwrap();
        let mut st = AppState::build(&cfg, &dir, "t".into(), 0).unwrap();
        st.engine = Some(crate::actor::spawn(Engine::new(&st.topo)));
        Arc::new(st)
    }

    async fn call(
        st: &Arc<AppState>,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> Response {
        let mut b = HttpRequest::builder().method(method).uri(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        handle(State(Arc::clone(st)), b.body(Body::from("{}")).unwrap()).await
    }

    fn entries(st: &AppState) -> Vec<LogEntry> {
        st.log
            .lock()
            .unwrap()
            .query(&log::Filter::default(), 1000)
            .entries
    }

    #[tokio::test]
    async fn each_request_makes_exactly_one_entry_and_one_event() {
        let st = app();
        let mut rx = st.events.subscribe();
        let reqs: [Req; 7] = [
            ("GET", "/users/1", &[]),
            ("GET", "/users", &[]),
            ("GET", "/nope", &[]),
            ("DELETE", "/users/1", &[]),
            ("GET", "/users/2", &[("x-kaanal-scenario", "bogus")]),
            ("GET", "/users/3", &[("x-kaanal-scenario", "down")]),
            ("OPTIONS", "/users/3", &[]),
        ];
        let mut ids = Vec::new();
        for (m, u, h) in reqs {
            let r = call(&st, m, u, h).await;
            if m != "OPTIONS" {
                ids.push(r.headers()["x-request-id"].to_str().unwrap().to_string());
            }
        }
        let logged = entries(&st);
        let mut events = Vec::new();
        while let Ok(BusEvent::Request(e)) = rx.try_recv() {
            events.push(e);
        }
        assert_eq!((logged.len(), events.len(), ids.len()), (6, 6, 6));
        assert_eq!(
            logged.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
            ids,
            "one entry per response, in order"
        );
        assert_eq!(events.iter().map(|e| e.id.clone()).collect::<Vec<_>>(), ids);
        ids.dedup();
        assert_eq!(ids.len(), 6);
    }

    #[tokio::test]
    async fn unmatched_and_bad_scope_are_logged_with_rejected() {
        let st = app();
        let mut rx = st.events.subscribe();
        call(&st, "GET", "/nope", &[]).await;
        call(&st, "DELETE", "/users/1", &[]).await;
        call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "bogus")]).await;
        let got = entries(&st);
        let shape: Vec<_> = got
            .iter()
            .map(|e| (e.status, e.rejected, e.endpoint.as_deref(), e.hops.len()))
            .collect();
        assert_eq!(
            shape,
            [
                (404, Some("unmatched"), None, 0),
                (405, Some("unmatched"), None, 0),
                (400, Some("bad-scope"), Some("GET /users/{id}"), 0),
            ]
        );
        assert!(
            got.iter()
                .all(|e| e.trace.is_empty() && e.failed_node.is_none())
        );
        for want in ["unmatched", "unmatched", "bad-scope"] {
            let Ok(BusEvent::Request(e)) = rx.try_recv() else {
                panic!("an event per rejection")
            };
            assert_eq!(e.rejected, Some(want));
            assert_eq!(e.nodes.len(), 0);
        }
    }

    #[tokio::test]
    async fn a_rejected_request_does_not_touch_the_counter() {
        let st = app();
        let count =
            |st: &AppState| -> u64 { st.counters.iter().map(|c| c.load(Ordering::Relaxed)).sum() };
        call(&st, "GET", "/users/1", &[]).await;
        let before = count(&st);
        call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "bogus")]).await;
        call(&st, "GET", "/nope", &[]).await;
        call(&st, "DELETE", "/users/1", &[]).await;
        assert_eq!(count(&st), before);
        assert_eq!(entries(&st).len(), 4);
    }

    #[tokio::test]
    async fn secrets_are_never_stored() {
        let st = app();
        let mut rx = st.events.subscribe();
        let r = HttpRequest::builder()
            .method("GET")
            .uri("/users/1?token=secret-query-789")
            .header("authorization", "Bearer secret-header-456")
            .header("x-custom", "secret-header-456")
            .header("x-request-id", "client-7")
            .body(Body::from("{\"password\":\"secret-body-123\"}"))
            .unwrap();
        handle(State(Arc::clone(&st)), r).await;
        let mut seen = serde_json::to_string(&entries(&st)).unwrap();
        while let Ok(BusEvent::Request(e)) = rx.try_recv() {
            seen.push_str(&serde_json::to_string(&e).unwrap());
        }
        for secret in ["secret-body-123", "secret-header-456", "secret-query-789"] {
            assert!(!seen.contains(secret), "{secret} leaked: {seen}");
        }
        assert!(seen.contains("client-7") && seen.contains("\"path\":\"/users/1\""));
    }

    #[tokio::test]
    async fn pinned_request_keeps_chain_error() {
        let st = app();
        let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hidden")]).await;
        assert_eq!(r.status(), 200);
        let e = &entries(&st)[0];
        assert_eq!(e.pin, Some("success"));
        assert_eq!(e.failed_node, None);
        let c = e.chain_error.as_ref().expect("the pin hid a chain error");
        assert_eq!(
            (c.node.as_str(), c.status, c.kind, c.cause),
            ("db", 503, "status", "chaos")
        );
        assert!(
            e.trace.ends_with("db.error=503,pinned=success"),
            "{}",
            e.trace
        );
        assert_eq!(e.flags, ["FI", "PN"]);
        // Unpinned, the same failure is the client's.
        call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "down")]).await;
        let e = &entries(&st)[1];
        assert_eq!((e.status, e.failed_node.as_deref()), (503, Some("db")));
        assert_eq!(e.chain_error.as_ref().map(|c| c.status), Some(503));
    }

    #[tokio::test]
    async fn out_of_order_completion_lands() {
        let st = app();
        let slow = tokio::spawn({
            let st = Arc::clone(&st);
            async move {
                call(&st, "GET", "/users/9", &[("x-kaanal-scenario", "lag")])
                    .await
                    .status()
            }
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        for _ in 0..3 {
            call(&st, "GET", "/users/1", &[]).await;
        }
        assert_eq!(entries(&st).len(), 3, "the slow request is still in flight");
        assert_eq!(slow.await.unwrap(), 200);
        let got = entries(&st);
        let last = got.last().unwrap();
        assert_eq!(
            (last.path.as_str(), last.seq, last.done),
            ("/users/9", 1, 4)
        );
        assert!(got[..3].iter().all(|e| e.seq > 1));
        // The latency chaos is attributed to the node's own hop, and flagged.
        assert_eq!(last.flags, ["DI"]);
        let db = last.hops.iter().find(|h| h.kind == "op").unwrap();
        assert_eq!(db.why.as_deref(), Some("chaos +300ms"));
    }

    #[tokio::test]
    async fn concurrent_requests_all_recorded_once() {
        let st = app();
        let tasks: Vec<_> = (0..50)
            .map(|i| {
                let st = Arc::clone(&st);
                tokio::spawn(async move {
                    let uri = if i % 5 == 0 { "/nope" } else { "/users/1" };
                    call(&st, "GET", uri, &[]).await.status()
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
        let got = entries(&st);
        assert_eq!(got.len(), 50);
        let mut seqs: Vec<_> = got.iter().map(|e| e.seq).collect();
        seqs.sort_unstable();
        assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
        let mut done: Vec<_> = got.iter().map(|e| e.done).collect();
        done.sort_unstable();
        assert_eq!(done, (1..=50).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn reset_empties_the_ring_but_ids_keep_counting() {
        let st = app();
        call(&st, "GET", "/users/1", &[]).await;
        crate::control::reset(State(Arc::clone(&st))).await.unwrap();
        assert_eq!(entries(&st).len(), 0);
        let r = call(&st, "GET", "/users/1", &[]).await;
        assert!(
            r.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .ends_with("-0002")
        );
        assert_eq!(entries(&st).len(), 1);
    }

    #[tokio::test]
    async fn engine_stopped_is_logged_with_an_id() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let st = Arc::new(
            AppState::build(&Config::from_yaml(YAML).unwrap(), &dir, "t".into(), 0).unwrap(),
        );
        let mut rx = st.events.subscribe();
        let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hang")]).await;
        assert_eq!(r.status(), 500);
        let id = r.headers()["x-request-id"].to_str().unwrap().to_string();
        let got = entries(&st);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].id.as_str(), got[0].rejected),
            (id.as_str(), Some("engine-stopped"))
        );
        assert_eq!(got[0].endpoint.as_deref(), Some("GET /users/{id}"));
        assert_eq!(got[0].scope.as_deref(), Some("hang"));
        let Ok(BusEvent::Request(e)) = rx.try_recv() else {
            panic!("one event")
        };
        assert_eq!(e.scope.as_deref(), Some("hang"));
        assert_eq!(
            (e.id, e.rejected, e.status),
            (id, Some("engine-stopped"), 500)
        );
        assert!(rx.try_recv().is_err(), "exactly one event");
    }

    #[tokio::test]
    async fn a_dropped_request_leaves_one_client_gone_entry() {
        let st = app();
        let mut rx = st.events.subscribe();
        // The hang holds the request for the node's 150 ms timeout; the client gives up at 40 ms.
        let gone = tokio::time::timeout(
            Duration::from_millis(40),
            call(&st, "POST", "/orders", &[("x-kaanal-scenario", "hang")]),
        )
        .await;
        assert!(gone.is_err(), "the request was still running");
        let got = entries(&st);
        assert_eq!(got.len(), 1);
        let e = &got[0];
        assert_eq!((e.status, e.rejected), (499, Some("client-gone")));
        assert_eq!(e.endpoint.as_deref(), Some("POST /orders"));
        assert_eq!(e.scope.as_deref(), Some("hang"));
        assert_eq!((e.hops.len(), e.trace.as_str()), (0, ""));
        let Ok(BusEvent::Request(ev)) = rx.try_recv() else {
            panic!("one event")
        };
        assert_eq!(
            (ev.id.as_str(), ev.rejected),
            (e.id.as_str(), Some("client-gone"))
        );
        // The engine finishes the abandoned plan later; nothing is logged twice.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(entries(&st).len(), 1);
        assert!(rx.try_recv().is_err());
        // A request that completes is pushed once, not also as client-gone.
        let r = call(&st, "GET", "/users/1", &[]).await;
        assert_eq!(r.status(), 200);
        let got = entries(&st);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].rejected, None);
    }

    #[tokio::test]
    async fn a_hang_is_never_reported_as_injected_delay() {
        let st = app();
        let r = call(&st, "POST", "/orders", &[("x-kaanal-scenario", "hang")]).await;
        assert_eq!(r.status(), 504);
        let e = &entries(&st)[0];
        // The node's own `timeout:` ends the hang, so the engine calls it a deadline; either way
        // the long hop is not injected latency (the FI-only case is in `record.rs`).
        assert_eq!(e.flags, ["DT"]);
        assert!(e.hops.iter().all(|h| h.why.is_none()), "{:?}", e.hops);
        assert_eq!(e.chain_error.as_ref().unwrap().kind, "timeout");
    }

    #[tokio::test]
    async fn injected_latency_is_attributed_on_nodes_caches_and_endpoints() {
        let st = app();
        call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "eplag")]).await;
        call(&st, "GET", "/users", &[("x-kaanal-scenario", "cachelag")]).await;
        let got = entries(&st);
        let ep = got[0].hops.iter().find(|h| h.kind == "delay").unwrap();
        assert_eq!(ep.why.as_deref(), Some("chaos +20ms"));
        assert_eq!(got[0].flags, ["DI"]);
        let hit = got[1].hops.iter().find(|h| h.kind == "hit").unwrap();
        assert_eq!(hit.why.as_deref(), Some("chaos +40ms"));
        assert_eq!(got[1].flags, ["DI"]);
        let op = got[0].hops.iter().find(|h| h.kind == "op").unwrap();
        assert_eq!(op.op, Some("read"));
    }

    #[tokio::test]
    async fn a_pin_over_a_reset_keeps_the_reset_kind() {
        let st = app();
        let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hreset")]).await;
        assert_eq!(r.status(), 200);
        let e = &entries(&st)[0];
        let c = e.chain_error.as_ref().unwrap();
        assert_eq!((c.node.as_str(), c.kind, c.cause), ("db", "reset", "chaos"));
        assert!(e.trace.contains("db.error=reset"), "{}", e.trace);
    }

    #[tokio::test]
    async fn long_paths_and_methods_are_capped_in_the_log() {
        let st = app();
        let path = format!("/{}é", "x".repeat(3000));
        call(&st, "GET", &path, &[]).await;
        call(&st, "PROPFINDPROPFINDPROPFIND", "/users/1", &[]).await;
        let got = entries(&st);
        assert!(got[0].path.len() <= 2048 && got[0].path.ends_with('…'));
        assert!(got[0].path.starts_with("/xxx"));
        assert_eq!(got[0].status, 404);
        assert!(got[1].method.len() <= 16 && got[1].method.ends_with('…'));
        // Short values are kept whole.
        call(&st, "GET", "/users/1", &[]).await;
        assert_eq!(entries(&st)[2].path, "/users/1");
    }

    /// Paused time: the engine's 60 s `HANG_CAP` sleep auto-advances instead of taking a minute.
    #[tokio::test(start_paused = true)]
    async fn a_hang_on_a_node_without_a_timeout_is_fault_injection() {
        let st = app();
        let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "dbhang")]).await;
        assert_eq!(r.status(), 504);
        let e = &entries(&st)[0];
        assert_eq!(e.flags, ["FI"]);
        assert!(e.hops.iter().all(|h| h.why.is_none()), "{:?}", e.hops);
        let c = e.chain_error.as_ref().unwrap();
        assert_eq!((c.kind, c.cause), ("timeout", "chaos"));
    }
}
