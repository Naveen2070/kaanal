//! The control API handlers and the security guard that stands in front of them.
//!
//! Every `/__kaanal/*` request passes [`guard`] before any handler or extractor
//! runs: Host must be loopback with the bound port, and every route except
//! `health`/`ui` needs the bearer token.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, ErrorKind, MAX_SCENARIOS, PathSeg, Resolved,
    ScenarioConfig, Source, endpoint_key, parse_scenario, resolve_scenario, valid_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::Instant;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tokio_stream::{Stream, StreamExt};

use crate::chaos_lease::{self, Leases};
use crate::generation::{Gen, Generation};
use crate::scenarios::{self, WriteError};
use crate::state::{AppState, BusEvent, Control, Effective};
mod admin;
mod chaos;
mod draft;
mod guard;
mod lease;
mod save;
mod scenario;
#[cfg(test)]
mod tests;
mod view;

pub use admin::{events, reset, shutdown, ui};
pub use chaos::{clear_keyed, clear_node, get_chaos, set_chaos, set_chaos_keyed, topology};
pub use draft::{apply_draft, get_draft, put_draft, save_draft, validate_draft};
pub use guard::guard;
pub use lease::lease_task;
pub use save::save_scenario;
// What only the unit tests reach by `super::`.
#[cfg(test)]
use chaos::ChaosPatch;
#[cfg(test)]
use guard::{ct_eq, host_allowed, query_token};
#[cfg(test)]
use lease::{expire_due, next_deadline};
use save::scenario_data;
use scenario::parse_ttl;
pub(crate) use scenario::pins_json;
pub use scenario::{
    activate, clear_scenarios, deactivate, delete_scenario, get_active, get_scenario,
    list_scenarios, put_scenario, set_active,
};
pub(crate) use view::key_label;
use view::{
    active_json, effective_entries, effective_json, key_fields, keyed_json, layers_json, place,
    source_name, unknown,
};

/// **Private.** Uniform JSON error shape returned by handlers: `(status, {"error": msg})`.
pub(crate) type Reject = (StatusCode, Json<Value>);

// limitation: error messages echo the node name from the URL path (`unknown node '...'`) unsanitized.
// It is JSON-encoded, so the browser UI is safe, and the CLI replaces control characters before printing.
// Other clients that print it to a terminal must do the same.
pub(crate) fn reject(code: StatusCode, msg: &str) -> Reject {
    (code, Json(json!({ "error": msg })))
}

/// **Private.** [`lock`], for a handler that resolved something against `g`: 409 when an Apply
/// has put another generation in force since the handler took it. The swap happens under this
/// same lock, so once it is held the answer cannot change, and a topology-bound id the handler
/// is about to store can never land in a newer generation's control state.
pub(crate) fn lock_gen<'a>(
    st: &'a AppState,
    g: &Generation,
) -> Result<MutexGuard<'a, Control>, Reject> {
    let c = lock(st);
    match st.generation() {
        Some(cur) if Arc::as_ptr(&cur) == std::ptr::from_ref(g) => Ok(c),
        _ => Err(reject(
            StatusCode::CONFLICT,
            "the system changed while this request ran; retry",
        )),
    }
}

/// **Private.** The control state and the generation in force, as one consistent pair: locked
/// first, then read, and an Apply swaps under the same lock. For the reads that pair the two
/// (`GET /chaos`, `GET /scenarios/active`, `GET /scenarios/{name}`): they never answer 409 for a
/// swap, they answer from whichever generation is in force once they hold the lock. 409 only
/// when there is none.
pub(crate) fn locked(st: &AppState) -> Result<(MutexGuard<'_, Control>, Arc<Generation>), Reject> {
    let c = lock(st);
    let g = st
        .generation()
        .ok_or_else(|| reject(StatusCode::CONFLICT, "no system applied yet; apply a draft"))?;
    Ok((c, g))
}

/// **Private.** The control state, locked. The guard must never live across an `.await`
/// (the handler futures would stop being `Send`, so the compiler enforces it).
///
/// A poisoned lock is recovered, not propagated: a panic in one handler must not take the whole
/// control plane down. `commit` restores its backup when the closure or the publish returns an
/// error, **not** when it panics: a panic mid-write leaves a half-written `Control`, which the
/// next successful write compiles and publishes. The request path is unaffected (it keeps the
/// last good published `Arc`); no handler closure panics in practice.
///
/// Only writers and the scenario GETs (`list_scenarios`, `get_scenario`, whose data lives in
/// the set) take this mutex. The request path never does: it reads the published `Arc`.
pub(crate) fn lock(st: &AppState) -> MutexGuard<'_, Control> {
    st.control.lock().unwrap_or_else(PoisonError::into_inner)
}

/// **Private.** The one write path (S11): runs `f` on the locked control state, publishes the
/// compiled result and returns it. If `f` or the publish fails, the state is put back, so a bad
/// request changes nothing.
pub(crate) fn commit(
    st: &AppState,
    c: &mut MutexGuard<'_, Control>,
    f: impl FnOnce(&mut Control) -> Result<(), Reject>,
) -> Result<Arc<Effective>, Reject> {
    let backup = (**c).clone();
    let done = f(c).and_then(|()| {
        // A lease outlives neither its activation nor its scenario, nor its live chaos field.
        let ctl = &mut **c;
        ctl.leases.retain(|n, _| ctl.stack.active().contains(n));
        chaos_lease::prune(&mut ctl.chaos_leases, ctl.stack.live());
        st.publish(c)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))
    });
    if done.is_err() {
        **c = backup;
    }
    done
}

/// **Private.** Tells SSE subscribers the stack changed.
pub(crate) fn emit(st: &AppState, action: &'static str, name: Option<&str>, snap: &Effective) {
    let _ = st.events.send(BusEvent::Scenario {
        action,
        name: name.map(str::to_string),
        chaos_key: None,
        active: snap.stack.active().to_vec(),
    });
}

/// **Private.** Milliseconds from `now` to `deadline`, rounded up (0 once past).
pub(crate) fn remaining_ms(deadline: Instant, now: Instant) -> u64 {
    let ns = deadline.saturating_duration_since(now).as_nanos();
    u64::try_from(ns.div_ceil(1_000_000)).unwrap_or(u64::MAX)
}
