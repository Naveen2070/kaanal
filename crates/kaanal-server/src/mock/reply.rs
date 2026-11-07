//! Building the answer: chaos failures, pinned states, the success body, wire failures (reset, malformed) and CORS.

use super::{
    AppState, Body, Bytes, ChainResult, Cut, Duration, FailKind, FinishedHop, Generation,
    HeaderValue, Outcome, PinState, Read, Response, Route, StatusCode, Topology, Value, auth,
    header, json, pins, resolve_error, response,
};

/// **Private.** What a request answers: status and body, the failing node, a `Retry-After` and,
/// for a wire failure, the broken response itself.
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Value,
    pub(super) failed: Option<(String, FailKind)>,
    pub(super) retry: Option<Duration>,
    pub(super) wire: Option<Response>,
}

/// **Private.** The request line and path parameters a reply is built from.
pub(super) struct Ask<'a> {
    pub(super) method: &'a str,
    pub(super) path: &'a str,
    pub(super) params: &'a [(String, String)],
    /// The decoded query pairs (D3 paging reads them).
    pub(super) query: &'a [(String, String)],
}

/// **Private.** The reply of a request with no pin: the chain's verdict decides. Only `Ok` applies
/// a collection operation; a wire failure shows a preview of it, which changes nothing.
pub(super) fn chaos_reply(
    st: &AppState,
    g: &Generation,
    route: &Route,
    result: &ChainResult,
    ask: &Ask,
    read: Read,
    trace: &str,
) -> Reply {
    let ep = &route.endpoint;
    let name = |id: usize| g.topo.nodes[id].name.as_str();
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
                && ep.is_wire_node(&g.topo, *node);
            let (mapped, body) = resolve_error(&ep.error_map, name(*node), *kind, *status);
            // An error_map entry by node name or by kind is the user's stated intent and beats the wire behaviour (Z7).
            // Same string lookup as `resolve_error`: a node named `reset` also counts as a kind key for other nodes.
            let overridden =
                ep.error_map.contains_key(name(*node)) || ep.error_map.contains_key(kind.as_str());
            let failed = Some((name(*node).to_string(), *kind));
            // The would-be reply, built only for a wire failure (a collection previews, never applies).
            let would = (at_wire && !overridden).then(|| match read {
                (_, Some(c)) => (c.preview(&st.collections), true),
                (login, None) => (answer(st, g, route, ask, (login, None)), false),
            });
            // A collection reply with no body to break (a 204, a HEAD) would read as a clean
            // success the store contradicts; it fails as at a non-wire node instead. A collection
            // never answers 304; it is listed because it is bodyless like the others.
            let bodyless = would.as_ref().is_some_and(|((status, _), coll)| {
                *coll && (ask.method == "HEAD" || matches!(status, 204 | 304))
            });
            let wire = would.filter(|_| !bodyless);
            if let Some((ok, _)) = wire {
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
            let (status, body) = answer(st, g, route, ask, read);
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
pub(super) fn pinned_reply(
    st: &AppState,
    g: &Generation,
    route: &Route,
    ask: &Ask,
    read: Read,
    out: &Outcome,
) -> Reply {
    let (status, body, retry) = pinned_response(st, g, route, ask, read, out);
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
/// 429 and 503. Never `error_map`, never a wire failure. Only the states that answer normally
/// apply a collection operation; `empty` and `error` change nothing.
pub(super) fn pinned_response(
    st: &AppState,
    g: &Generation,
    route: &Route,
    ask: &Ask,
    read: Read,
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
            let owner = ep.owner.map_or("", |id| g.topo.nodes[id].name.as_str());
            let seed = pins::route_seed(g.seed, owner, ask.path);
            let opts = g.options_for_owner(ep.owner);
            let mut body =
                pins::empty_form(ep, route.schema.as_ref(), seed, opts).unwrap_or(Value::Null);
            // P9: a `states.empty.body` is returned verbatim; only a derived body gets the params.
            if ep.states.empty.as_ref().is_none_or(|o| o.body.is_none()) {
                kaanal_data::apply_params(&mut body, ask.params);
            }
            (status, body, None)
        }
        PinState::Loading | PinState::Slow | PinState::Success => {
            let (status, body) = answer(st, g, route, ask, read);
            (status, body, None)
        }
    }
}

/// **Private.** The chaos body of a failure that answers as JSON. Kind `status` keeps the
/// original two-field shape; any other kind names itself.
pub(super) fn default_error_body(node: &str, kind: FailKind) -> Value {
    if kind == FailKind::Status {
        json!({ "error": "chaos", "node": node })
    } else {
        json!({ "error": "chaos", "node": node, "kind": kind.as_str() })
    }
}

/// **Private.** What a route answers when nothing fails: the `override`, else the generated body
/// (one page of it, or a 400 for a bad paging parameter, on a paged route).
pub(super) fn success(g: &Generation, route: &Route, ask: &Ask) -> (u16, Value) {
    let ep = &route.endpoint;
    if let Some(ov) = &ep.response {
        return (ov.status, ov.body.clone());
    }
    let owner = ep.owner.map_or("", |id| g.topo.nodes[id].name.as_str());
    if let Some(spec) = &route.paging {
        return super::paging::page(g, route, spec, owner, ask);
    }
    (
        route.status,
        generated_body(g, route, owner, ask.path, ask.params),
    )
}

/// **Private.** What the route answers when nothing fails: its collection operation (applied here
/// and nowhere else), a built-in responder (it owns its status codes; the route's 200 and schema
/// do not apply), else [`success`].
pub(super) fn answer(
    st: &AppState,
    g: &Generation,
    route: &Route,
    ask: &Ask,
    (login, coll): Read,
) -> (u16, Value) {
    if let Some(c) = coll {
        return c.apply(&st.collections);
    }
    let ep = &route.endpoint;
    match ep.responder.filter(|_| ep.response.is_none()) {
        Some(r) => auth::respond(r, g.auth.as_ref(), login),
        None => success(g, route, ask),
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
// A bare-number body ("12") can cut to valid JSON ("1"); schema bodies are objects or arrays in practice.
pub(super) fn wire_failure(
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
pub(super) fn generated_body(
    g: &Generation,
    route: &Route,
    owner: &str,
    path: &str,
    params: &[(String, String)],
) -> Value {
    let Some(schema) = &route.schema else {
        return Value::Null;
    };
    // `seed:` (not the chain seed) keys the data: same seed → same data (NFR-10, D14).
    let seed = pins::route_seed(g.seed, owner, path);
    let opts = g.options_for_owner(route.endpoint.owner);
    let mut v = kaanal_data::generate(schema, None, seed, opts);
    kaanal_data::apply_params(&mut v, params);
    v
}

/// **Private.** P11: tokens and credential errors are never cached, whatever answered.
pub(super) fn no_store(r: &mut Response) {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

/// **Private.** Ordered unique node names the request touched, plus the failing node.
pub(super) fn touched_nodes(
    hops: &[FinishedHop],
    failed: Option<&str>,
    topo: &Topology,
) -> Vec<String> {
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
pub(super) fn with_cors(mut r: Response) -> Response {
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
        HeaderValue::from_static(
            "x-kaanal-trace, x-kaanal-scope, x-request-id, retry-after, x-total-count, link",
        ),
    );
    r
}
