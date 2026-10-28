//! The draft routes: `GET` and `PUT /__kaanal/draft`.

use super::*;
use crate::draft::MAX_DRAFT_BYTES;
use crate::draft::apply::{self as apply_mod, ApplyError};

/// **Private.** The draft as the API shows it: `{ text, dirty, generation }` (`generation` is 0
/// before the first Apply of a blank start).
fn view(st: &AppState) -> Json<Value> {
    let d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
    let generation = st.generation().map_or(0, |g| g.id);
    Json(json!({ "text": d.text, "dirty": d.dirty(), "generation": generation }))
}

/// `GET /__kaanal/draft` — the draft text, whether it differs from what is applied, and the
/// generation in force.
pub async fn get_draft(State(st): State<Arc<AppState>>) -> Json<Value> {
    view(&st)
}

/// The accepted body of `PUT /__kaanal/draft`.
#[derive(Deserialize)]
pub struct DraftBody {
    text: String,
}

/// `PUT /__kaanal/draft` — replaces the draft text and returns the new view. The text is not
/// validated (a half-typed file must be savable); `POST /draft/validate` does that.
///
/// # Errors
/// 400 for a body without `text`, or a text over 1 MiB (1048576 bytes); the draft is unchanged.
pub async fn put_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Json<DraftBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    if b.text.len() > MAX_DRAFT_BYTES {
        return Err(too_large(b.text.len()));
    }
    st.draft.lock().unwrap_or_else(PoisonError::into_inner).text = b.text;
    Ok(view(&st))
}

/// `POST /__kaanal/draft/validate` — builds the draft (or the `text` of the body, without storing
/// it) off to the side with the loader `serve` uses, and answers `{ ok, diagnostics, warnings }`.
/// It never swaps the system and never starts an engine. A problem in the draft is data (`200`,
/// `ok: false`), not an HTTP error.
///
/// # Errors
/// 400 for a body that is not `{ "text": ... }` or a text over 1 MiB.
pub async fn validate_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Option<Json<DraftBody>>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let body = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let text = match body {
        Some(Json(b)) if b.text.len() > MAX_DRAFT_BYTES => return Err(too_large(b.text.len())),
        Some(Json(b)) => b.text,
        None => st
            .draft
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .text
            .clone(),
    };
    let (dir, seed) = (st.dir.clone(), st.drawn_seed);
    let (key, port) = (st.auth_key.clone(), st.port);
    // reads spec and data files, so off the async threads
    let built = tokio::task::spawn_blocking(move || {
        crate::draft::apply::build_draft(&text, &dir, seed, key, port)
            .map(|b| (b.generation.warnings, b.generation.flagged))
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(Json(match built {
        Ok((warnings, flagged)) => {
            json!({ "ok": true, "diagnostics": [], "warnings": warnings, "flagged": flagged })
        }
        Err(d) => json!({ "ok": false, "diagnostics": [d], "warnings": [], "flagged": [] }),
    }))
}

/// **Private.** The 400 for a draft over the limit.
fn too_large(len: usize) -> Reject {
    reject(
        StatusCode::BAD_REQUEST,
        &format!("draft too large: {len} bytes (limit {MAX_DRAFT_BYTES})"),
    )
}

/// The accepted body of `POST /__kaanal/draft/apply`.
#[derive(Deserialize, Default)]
pub struct ApplyBody {
    timeout_ms: Option<u64>,
}

/// The default and the longest drain an Apply waits, in milliseconds.
const DEFAULT_DRAIN_MS: u64 = 10_000;
const MAX_DRAIN_MS: u64 = 60_000;

/// `POST /__kaanal/draft/apply` — builds the draft, waits (up to `timeout_ms`, default 10 s, at
/// most 60 s) for the mock requests in flight to finish while new ones wait, swaps the system and
/// lets traffic go on. Answers `{ ok, generation, kept, dropped, flagged }`.
///
/// # Errors
/// 400 when the draft does not build (with `diagnostics`) or `timeout_ms` is over 60 s; 409 when
/// requests did not drain in time (the old system keeps running, nothing changed).
pub async fn apply_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Option<Json<ApplyBody>>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let body = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let ms = body
        .and_then(|Json(b)| b.timeout_ms)
        .unwrap_or(DEFAULT_DRAIN_MS);
    if ms > MAX_DRAIN_MS {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            &format!("timeout_ms is at most {MAX_DRAIN_MS}"),
        ));
    }
    match apply_mod::apply(&st, Duration::from_millis(ms)).await {
        Ok(a) => Ok(Json(json!({
            "ok": true,
            "generation": a.generation,
            "kept": a.kept,
            "dropped": a.dropped,
            "flagged": a.flagged,
        }))),
        Err(ApplyError::Invalid(d)) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": d.message, "diagnostics": [d] })),
        )),
        Err(ApplyError::Timeout) => Err(reject(
            StatusCode::CONFLICT,
            "drain timed out; the old system keeps running",
        )),
        Err(ApplyError::Internal(m)) => Err(reject(StatusCode::INTERNAL_SERVER_ERROR, &m)),
    }
}
