//! One request in flight: the log entry it must leave, the identity it carries, and the paths that end a request early.

use super::{
    Answer, AppState, Arc, Ask, BusEvent, ChainResult, Duration, EffectiveChaos, Endpoint, Facts,
    Finished, HeaderValue, Instant, IntoResponse, Json, LogEntry, Outcome, PinState, Plan, Read,
    Reply, Request, Responder, Response, Route, Run, StatusCode, SystemTime, TraceParts,
    UNIX_EPOCH, Value, chaos_reply, header, json, log, no_store, pinned_reply, record, scope,
    touched_nodes,
};

/// **Private.** A mock request in flight. If the handler future is dropped (the client went away)
/// before it finished, the drop publishes a `client-gone` entry (status 499, no hops) and its
/// event, so every id ends in exactly one entry. Every normal exit takes `ident` first, which
/// disarms the guard; it never publishes twice.
pub(super) struct Flight {
    pub(super) st: Arc<AppState>,
    pub(super) ident: Option<Ident>,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) endpoint: Option<Arc<Endpoint>>,
}

impl Flight {
    /// Takes the request's id and starts its clock.
    pub(super) fn new(st: &Arc<AppState>, req: &Request, path: String) -> Self {
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
pub(super) struct Served<'a> {
    pub(super) route: &'a Route,
    pub(super) ident: Ident,
    pub(super) method: &'a str,
    pub(super) path: &'a str,
    pub(super) params: &'a [(String, String)],
    pub(super) query: &'a [(String, String)],
    pub(super) scope: Option<&'a str>,
    pub(super) chaos: &'a EffectiveChaos,
    pub(super) plan: &'a Plan,
    pub(super) pin: Option<&'a Outcome>,
    pub(super) fin: &'a Finished,
}

/// **Private.** Builds the response of a request the chain served, then logs it and tells the bus.
pub(super) fn finish(st: &AppState, read: Read, s: Served) -> Response {
    let Served {
        route,
        ident,
        method,
        path,
        params,
        query,
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
        query,
    };
    // P2: a pin wins over whatever the chain did; the chain still ran (its hops and delay are real).
    let Reply {
        status,
        body,
        failed,
        retry,
        wire,
    } = match pin {
        Some(o) => pinned_reply(st, route, &ask, read, o),
        None => chaos_reply(st, route, &fin.result, &ask, read, &header),
    };

    // D3: only a normal success of a paged route carries `X-Total-Count` and `Link`; a failure,
    // a wire break, a pinned `empty` or `error` and the 400 of a bad query do not.
    let paged = failed.is_none()
        && wire.is_none()
        && status == route.status
        && pin.is_none_or(|o| !matches!(o.state, PinState::Empty | PinState::Error));
    let mut resp = wire.unwrap_or_else(|| response(status, body, &header, retry));
    if paged {
        super::paging::add_headers(st, route, &ask, &mut resp);
    }
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
pub(super) struct Ident {
    pub(super) id: String,
    pub(super) client_id: Option<String>,
    pub(super) seq: u64,
    pub(super) started: Instant,
    /// The scope key once the header resolved, so a rejection or an abort after that keeps it.
    pub(super) scope: Option<String>,
}

impl Ident {
    /// The request facts for its log entry. `ms` is measured now, not simulated: it shows what a
    /// client really waited (timer granularity, load). The trace keeps the simulated per-hop times.
    pub(super) fn facts<'a>(
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
pub(super) fn tag(resp: &mut Response, id: &str) {
    if let Ok(v) = HeaderValue::from_str(id) {
        resp.headers_mut().insert("x-request-id", v);
    }
}

/// **Private.** Logs a request refused before any chain ran and tags its response. These paths
/// touch neither the route counter nor a seeded draw.
pub(super) fn rejected(
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
pub(super) fn publish(st: &AppState, entry: LogEntry, nodes: Vec<String>) {
    let event = record::event_of(&entry, nodes);
    st.push_log(entry);
    let _ = st.events.send(BusEvent::Request(event));
}

/// **Private.** The 404 for a path nothing serves.
pub(super) fn no_route(path: &str) -> Response {
    let body = Json(json!({ "error": "no route", "path": path }));
    (StatusCode::NOT_FOUND, body).into_response()
}

/// **Private.** The reply: status and JSON body (none for `null`), the trace header and, for a
/// 429, `Retry-After` in whole seconds rounded up (at least 1).
pub(super) fn response(status: u16, body: Value, trace: &str, retry: Option<Duration>) -> Response {
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
pub(super) fn engine_stopped() -> Response {
    let body = Json(json!({ "error": "engine stopped" }));
    (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
}
