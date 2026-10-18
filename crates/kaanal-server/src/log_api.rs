//! The log control routes: `GET /__kaanal/logs` and `GET /__kaanal/trace/{id}`. Both read the
//! ring under its lock only long enough to clone what they return.

use std::sync::{Arc, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::control::{Reject, reject};
use crate::log::{Filter, LOG_CAP, Lookup};
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 100;

/// **Private.** `limit` from the pairs: absent is the default; otherwise plain digits in `1..=LOG_CAP`.
fn limit_of(params: &[(String, String)]) -> Result<usize, Reject> {
    let Some((_, v)) = params.iter().find(|(k, _)| k == "limit") else {
        return Ok(DEFAULT_LIMIT);
    };
    v.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| v.parse::<usize>().ok())
        .flatten()
        .filter(|n| (1..=LOG_CAP).contains(n))
        .ok_or_else(|| {
            reject(
                StatusCode::BAD_REQUEST,
                &format!("parameter 'limit': expected an integer from 1 to {LOG_CAP}"),
            )
        })
}

/// `GET /__kaanal/logs` — the last `limit` (default 100, max 1000) finished requests matching
/// the filters, oldest completion first: `{ entries, oldest, newest, cap, evicted }`.
///
/// # Errors
/// 400 naming the parameter for an unknown, repeated, empty or malformed one.
pub async fn logs(
    State(st): State<Arc<AppState>>,
    q: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Result<Json<Value>, Reject> {
    let Query(params) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let limit = limit_of(&params)?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let filter =
        Filter::from_params(&params, now_ms).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let page = st
        .log
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .query(&filter, limit);
    Ok(Json(json!(page)))
}

/// `GET /__kaanal/trace/{id}` — one entry by our id, `last` (the latest to finish) or a client
/// `X-Request-Id`.
///
/// # Errors
/// 404 `evicted` or `unknown`; 400 for a key that is neither an id nor a valid client id, or
/// whose percent-decoding is not UTF-8.
pub async fn trace(
    State(st): State<Arc<AppState>>,
    key: Result<Path<String>, PathRejection>,
) -> Result<Json<Value>, Reject> {
    let Path(key) = key.map_err(|e| reject(e.status(), &e.body_text()))?;
    let found = st
        .log
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .find(&key);
    match found {
        Lookup::Found(e) => Ok(Json(json!(e))),
        Lookup::Evicted => Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "evicted",
                "detail": format!("older than the last {LOG_CAP} requests, or still in flight"),
            })),
        )),
        Lookup::Unknown => Err((StatusCode::NOT_FOUND, Json(json!({ "error": "unknown" })))),
        Lookup::Malformed => Err(reject(
            StatusCode::BAD_REQUEST,
            "expected a request id like r04f2-0042, 'last' or a client X-Request-Id",
        )),
    }
}
