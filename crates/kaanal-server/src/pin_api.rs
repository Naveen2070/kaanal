//! The pin control routes: `GET|PUT|DELETE /__kaanal/pins` and `POST /__kaanal/cycle`.
//! Every write goes through `lock`/`commit`/`emit`, like the chaos routes, so it is atomic
//! with scenario writes (S11) and the publish clears the scoped cache.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use kaanal_core::{Endpoint, PinEntry, PinSource, PinState, endpoint_key, validate_entry};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::control::{Reject, commit, emit, lock, lock_gen, reject};
use crate::generation::{Gen, Generation};
use crate::state::AppState;

/// The body of `PUT /pins` and `POST /cycle` (`state` is optional only for a cycle).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinBody {
    endpoint: String,
    state: Option<String>,
    status: Option<u16>,
    delay: Option<String>,
}

/// The query of `DELETE /pins`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinQuery {
    endpoint: Option<String>,
}

fn bad(m: &str) -> Reject {
    reject(StatusCode::BAD_REQUEST, m)
}

/// **Private.** `{state, status?, delay_ms?}` of an entry.
fn entry_json(e: &PinEntry) -> serde_json::Map<String, Value> {
    let mut o = serde_json::Map::new();
    o.insert("state".into(), json!(e.state.name()));
    if let Some(s) = e.status {
        o.insert("status".into(), json!(s));
    }
    if let Some(d) = e.delay {
        o.insert(
            "delay_ms".into(),
            json!(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        );
    }
    o
}

/// **Private.** The normalized key of a known, pinnable endpoint: 400 for a malformed key, a
/// `HEAD` made from its `GET` or a route with no entry node; 404 for an unknown one.
fn pinnable<'a>(g: &'a Generation, raw: &str) -> Result<(String, &'a Endpoint), Reject> {
    let key = endpoint_key(raw).map_err(|m| bad(&m))?;
    let ep = g
        .endpoints
        .get(&key)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown endpoint '{key}'")))?;
    if ep.synthesized {
        return Err(bad(&format!(
            "'{key}' is a HEAD made from its GET; pin the GET route"
        )));
    }
    if ep.entry_node.is_none() {
        return Err(bad(&format!(
            "endpoint '{key}' has no entry node (a static route needs a gateway)"
        )));
    }
    Ok((key, ep))
}

/// **Private.** The state, status and delay of a body, checked for the state.
fn entry_of(
    g: &Generation,
    key: &str,
    ep: &Endpoint,
    state: PinState,
    b: &PinBody,
) -> Result<PinEntry, Reject> {
    let delay = b
        .delay
        .as_deref()
        .map(humantime::parse_duration)
        .transpose()
        .map_err(|e| bad(&format!("delay: {e}")))?;
    validate_entry(state, b.status, delay).map_err(|m| bad(&m))?;
    if state == PinState::Empty && !g.empty_ok_for(ep) {
        return Err(bad(&format!(
            "no empty form for '{key}'; add states.empty.body to its endpoints entry"
        )));
    }
    Ok(PinEntry {
        state,
        status: b.status,
        delay,
    })
}

/// `GET /__kaanal/pins` — `{ live: [..], effective: { key: {.., source} } }`.
pub async fn get_pins(State(st): State<Arc<AppState>>) -> Json<Value> {
    let snap = st.snapshot();
    let live: Vec<Value> = snap
        .stack
        .live_pins()
        .iter()
        .map(|(k, e)| {
            let mut o = entry_json(e);
            o.insert("endpoint".into(), json!(k));
            Value::Object(o)
        })
        .collect();
    let effective: serde_json::Map<String, Value> = snap
        .pins
        .iter()
        .map(|(k, r)| {
            let mut o = entry_json(&r.entry);
            o.insert(
                "source".into(),
                match &r.source {
                    PinSource::Live => json!("live"),
                    PinSource::Scenario(n) => json!({ "scenario": n }),
                },
            );
            (k.clone(), Value::Object(o))
        })
        .collect();
    Json(json!({ "live": live, "effective": effective }))
}

/// `PUT /__kaanal/pins` — sets the live pin of one endpoint; answers it like a cycle does.
///
/// # Errors
/// 404 unknown endpoint; 400 for a bad value, a `HEAD`, no entry node or `empty` with no form.
pub async fn put_pin(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    payload: Result<Json<PinBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let (key, ep) = pinnable(&g, &b.endpoint)?;
    let state = b
        .state
        .as_deref()
        .and_then(PinState::parse)
        .ok_or_else(|| bad("state must be loading, empty, error, slow or success"))?;
    let entry = entry_of(&g, &key, ep, state, &b)?;
    let mut c = lock_gen(&st, &g)?;
    let snap = commit(&st, &mut c, |c| {
        c.stack.live_pins_mut().insert(key.clone(), entry);
        Ok(())
    })?;
    emit(&st, "pin", Some(&key), &snap);
    Ok(answer(&key, &entry))
}

fn answer(key: &str, e: &PinEntry) -> Json<Value> {
    let mut o = entry_json(e);
    o.insert("endpoint".into(), json!(key));
    Json(Value::Object(o))
}

/// `DELETE /__kaanal/pins?endpoint=EP` — removes that live pin (204, also when absent); with
/// no query, clears every live pin. Scenario pins stay.
///
/// # Errors
/// 400 for a malformed endpoint.
pub async fn delete_pins(
    State(st): State<Arc<AppState>>,
    q: Result<Query<PinQuery>, QueryRejection>,
) -> Result<StatusCode, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let key = q
        .endpoint
        .as_deref()
        .map(endpoint_key)
        .transpose()
        .map_err(|m| bad(&m))?;
    let mut c = lock(&st);
    let live = c.stack.live_pins();
    // Nothing to remove: no publish, no scoped-cache clear, no event.
    if key
        .as_deref()
        .map_or(live.is_empty(), |k| live.get(k).is_none())
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    let snap = commit(&st, &mut c, |c| {
        match &key {
            Some(k) => {
                c.stack.live_pins_mut().remove(k);
            }
            None => c.stack.live_pins_mut().clear(),
        }
        Ok(())
    })?;
    emit(&st, "pin", key.as_deref(), &snap);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /__kaanal/cycle` — steps the live pin of an endpoint to the next state (loading,
/// empty, error, slow, success, wrapping; `empty` skipped where there is no empty form), or
/// pins `state` when given. Answers `{ endpoint, state, status?, delay_ms? }`. Any 400 changes
/// nothing.
///
/// # Errors
/// As [`put_pin`].
pub async fn cycle(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    payload: Result<Json<PinBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let (key, ep) = pinnable(&g, &b.endpoint)?;
    let ok = g.empty_ok_for(ep);
    let allowed = |s: PinState| ok || s != PinState::Empty;
    let wanted = match b.state.as_deref() {
        None => None,
        Some(s) => Some(
            PinState::parse(s)
                .ok_or_else(|| bad("state must be loading, empty, error, slow or success"))?,
        ),
    };
    let mut c = lock_gen(&st, &g)?;
    let state = wanted
        .unwrap_or_else(|| PinState::next(c.stack.live_pins().get(&key).map(|e| e.state), allowed));
    let entry = entry_of(&g, &key, ep, state, &b)?;
    let snap = commit(&st, &mut c, |c| {
        c.stack.live_pins_mut().insert(key.clone(), entry);
        Ok(())
    })?;
    emit(&st, "cycle", Some(&key), &snap);
    Ok(answer(&key, &entry))
}
