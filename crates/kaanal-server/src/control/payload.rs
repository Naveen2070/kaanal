//! The payload routes (step 8A): `GET|PUT|DELETE /__kaanal/payload` and
//! `GET /__kaanal/payload/preview`. An edit is a live `override:` for one route, applied in
//! `mock::finish` below chaos and pins. Writes take the control lock, so an Apply (which prunes
//! under the same lock) never meets a half-set edit.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use kaanal_core::config::Override;
use kaanal_core::scenario::endpoint_key;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Reject, lock_gen, reject};
use crate::body::{ReadError, read_capped};
use crate::generation::Gen;
use crate::payload::PayloadError;
use crate::state::AppState;

/// The body of `PUT /payload`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutBody {
    route: String,
    status: Option<u16>,
    body: Option<Value>,
    headers: Option<BTreeMap<String, String>>,
}

/// The query of `DELETE /payload` and `GET /payload/preview`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteQuery {
    route: String,
}

/// **Private.** The JSON of one edit.
fn edit_json(route: &str, o: &Override) -> Value {
    json!({ "route": route, "status": o.status, "body": o.body, "headers": o.headers })
}

/// **Private.** `422 { error, field }`; the field is the one the message starts with.
fn unprocessable(msg: &str, field: &str) -> Reject {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": msg, "field": field })),
    )
}

/// **Private.** A [`PayloadError`] as a reply.
fn refused(e: PayloadError) -> Reject {
    match e {
        PayloadError::NoRoute(m) => reject(StatusCode::NOT_FOUND, &m),
        PayloadError::Invalid(m) => {
            let field = m.split(':').next().unwrap_or("route");
            let field = if matches!(field, "status" | "body" | "headers") {
                field
            } else {
                "route"
            };
            unprocessable(&m, field)
        }
        PayloadError::Synthesized(m) | PayloadError::Collection(m) | PayloadError::BuiltIn(m) => {
            unprocessable(&m, "route")
        }
    }
}

/// `GET /__kaanal/payload` — the live edits, in route order: `{ edits: [{ route, status, body,
/// headers }] }`.
pub async fn list(State(st): State<Arc<AppState>>) -> Json<Value> {
    let edits: Vec<Value> = st.payloads().iter().map(|(k, o)| edit_json(k, o)).collect();
    Json(json!({ "edits": edits }))
}

/// `PUT /__kaanal/payload` — sets the live edit of one route (`status` 200, `body` null and
/// `headers` empty when left out); 204.
///
/// # Errors
/// 400 for malformed JSON; 404 unknown route; 409 when an Apply swapped the system meanwhile;
/// 413 over the body cap; 422 with `field` for a refused value or route.
pub async fn put(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    req: Request,
) -> Result<StatusCode, Reject> {
    let bytes = read_capped(req, Override::MAX_BODY + 4096)
        .await
        .map_err(|e| match e {
            ReadError::TooLarge => reject(StatusCode::PAYLOAD_TOO_LARGE, "body is too large"),
            ReadError::Broken => reject(StatusCode::BAD_REQUEST, "the body could not be read"),
        })?;
    let b: PutBody = serde_json::from_slice(&bytes)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let ov = Override {
        status: b.status.unwrap_or(200),
        body: b.body.unwrap_or(Value::Null),
        headers: b.headers.unwrap_or_default(),
    };
    let text = format!("answers {}", ov.status);
    let _c = lock_gen(&st, &g)?;
    let key = st.set_payload(&g, &b.route, ov).map_err(refused)?;
    st.live.mark("payload", &format!("{key} {text}"));
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /__kaanal/payload?route=KEY` — back to normal; 204.
///
/// # Errors
/// 400 for a missing or malformed `route`; 404 when the route has no edit.
pub async fn delete(
    State(st): State<Arc<AppState>>,
    q: Result<Query<RouteQuery>, QueryRejection>,
) -> Result<StatusCode, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let key = endpoint_key(&q.route).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let _c = super::lock(&st);
    if !st.clear_payload(&key) {
        return Err(reject(
            StatusCode::NOT_FOUND,
            &format!("'{key}' has no edit"),
        ));
    }
    st.live.mark("payload", &format!("{key} back to normal"));
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /__kaanal/payload/preview?route=KEY` — what the route answers with no edit:
/// `{ status, body, headers }` (the generated body, or the file's `override:`), a starting point
/// for the editor. Path parameters are filled with `1`.
///
/// # Errors
/// 400 for a missing or malformed `route`; 404 for an unknown one.
pub async fn preview(
    Gen(g): Gen,
    q: Result<Query<RouteQuery>, QueryRejection>,
) -> Result<Json<Value>, Reject> {
    let Query(q) = q.map_err(|e| reject(e.status(), &e.body_text()))?;
    let key = endpoint_key(&q.route).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let (status, body, headers) = crate::mock::preview(&g, &key)
        .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("unknown endpoint '{key}'")))?;
    Ok(Json(
        json!({ "status": status, "body": body, "headers": headers }),
    ))
}
