//! Reset, shutdown, the event stream and the embedded UI page.

use super::{
    AppState, Arc, BroadcastStream, BroadcastStreamRecvError, BusEvent, Bytes, Duration, Html,
    Infallible, IntoResponse, Json, KeepAlive, Ordering, PoisonError, Reject, Source, Sse,
    SseEvent, State, StatusCode, Stream, StreamExt, Value, WatchStream, commit, emit, header, json,
    lock, reject, scenario_data,
};

/// `POST /__kaanal/reset` — back to a clean slate for `beforeEach` (S18): deactivates every
/// scenario, **undefines the runtime-defined ones** (inline and file scenarios stay), clears the
/// live layer, zeroes every route counter, empties every collection and refills the engine's
/// rate-limit buckets, so a seeded run repeats.
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
    // a blank start has no system yet: nothing to zero
    if let Some(g) = st.generation() {
        for n in &g.counters {
            n.store(0, Ordering::Relaxed);
        }
        if let Some(e) = &g.engine {
            e.refill_buckets();
        }
    }
    st.log
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    st.collections.clear();
    st.clear_payloads();
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
            Ok(BusEvent::Applied { generation }) => SseEvent::default()
                .event("applied")
                .json_data(json!({ "generation": generation }))
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
        Html(include_str!("../ui.html")),
    )
}
