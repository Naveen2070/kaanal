//! `GET /__kaanal/openapi/skeleton`: a generated `OpenAPI` file for the draft's `endpoints:`.
//!
//! It reads the **draft**, not the applied system, so it works before the first Apply (a blank
//! start has endpoints to describe and no spec to carry them). Paths of an `endpoints:` file are
//! held to the project folder like any draft path (R1.71).

use super::{AppState, Arc, Json, PoisonError, Reject, State, StatusCode, Value, json, reject};
use kaanal_core::config::parse_endpoints;
use kaanal_core::diagnostics::keys;
use kaanal_core::{Config, EndpointsConfig, skeleton_openapi};

use crate::paths::{Trust, read_text, resolve};

/// **Private.** The skeleton text and how many operations it holds.
fn build(text: &str, dir: &std::path::Path) -> Result<(String, usize), Reject> {
    let cfg = Config::from_yaml(text).map_err(|e| {
        reject(
            StatusCode::BAD_REQUEST,
            &format!("the draft does not parse: {e}"),
        )
    })?;
    let from_file = matches!(cfg.endpoints, Some(EndpointsConfig::File(_)));
    let entries = match cfg.endpoints {
        None => indexmap::IndexMap::new(),
        Some(EndpointsConfig::Inline(m)) => m,
        Some(EndpointsConfig::File(p)) => {
            let file = resolve(dir, &p, Trust::Api, &keys(&["endpoints"]))
                .map_err(|e| reject(StatusCode::BAD_REQUEST, &e.msg))?;
            let src = read_text(&file).map_err(|e| {
                reject(
                    StatusCode::BAD_REQUEST,
                    &format!("the endpoints file cannot be read: {e}"),
                )
            })?;
            // an endpoints file is a draft-sized thing here: 16 MiB would be parsed into memory
            if src.len() > crate::draft::MAX_DRAFT_BYTES {
                return Err(reject(
                    StatusCode::BAD_REQUEST,
                    "the endpoints file is too large for a skeleton (at most 1 MiB)",
                ));
            }
            parse_endpoints(&src).map_err(|_| {
                reject(
                    StatusCode::BAD_REQUEST,
                    "the endpoints file is not a valid endpoints map",
                )
            })?
        }
    };
    let n = entries.len();
    let yaml = skeleton_openapi(&entries, "Kaanal skeleton").map_err(|m| {
        // a key read from a file is never echoed (R1.82): the file may not be an endpoints map
        let m = if from_file {
            "the endpoints file has a key that cannot be an endpoint (METHOD /path), a malformed path, or too many entries"
                .to_string()
        } else {
            m
        };
        reject(StatusCode::CONFLICT, &m)
    })?;
    Ok((yaml, n))
}

/// `GET /__kaanal/openapi/skeleton` — `{ text, endpoints }`: one operation per `endpoints:` key
/// of the draft, with its path parameters and a success response with an empty object schema.
/// Register it as a node's `spec:` so the entries apply to real routes.
///
/// # Errors
/// 400 when the draft does not parse or its endpoints file cannot be read; 409 when it has no
/// endpoints, or a key that is not `METHOD /path`.
pub async fn get_skeleton(State(st): State<Arc<AppState>>) -> Result<Json<Value>, Reject> {
    let text = st
        .draft
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .text
        .clone();
    let dir = st.dir.clone();
    // reads a file and builds a document: off the async threads, a few at a time (the permit
    // is held by the job itself)
    let permit = Arc::clone(&st.work).try_acquire_owned().map_err(|_| {
        reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many samples or skeletons at once; retry",
        )
    })?;
    let (yaml, n) = tokio::task::spawn_blocking(move || {
        let _held = permit;
        build(&text, &dir)
    })
    .await
    .map_err(|_| reject(StatusCode::INTERNAL_SERVER_ERROR, "the skeleton failed"))??;
    Ok(Json(json!({ "text": yaml, "endpoints": n })))
}
