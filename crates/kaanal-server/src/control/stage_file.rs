//! `POST /__kaanal/payload/stage-file` (step 8A leftover, RULES 111): writes a live edit into the
//! route's entry of the **endpoints file** the draft names (`endpoints: routes.yaml`), as the
//! draft's Stage does for an inline `endpoints:`. The draft holds only `kaanal.yaml`, so this one
//! writes the other file itself, at once, atomically, with the same comment-keeping splice; the
//! running system keeps its live edit until the next Apply reads the file.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use kaanal_core::config::{Override, parse_endpoints};
use kaanal_core::diagnostics::keys;
use kaanal_core::scenario::endpoint_key;
use kaanal_core::{Config, EndpointsConfig};
use serde_json::{Value, json};

use super::payload::{PutBody, refused};
use super::{Reject, lock_gen, reject};
use crate::body::{ReadError, read_capped};
use crate::draft::patch::{self, Op};
use crate::draft::save::{Change, commit};
use crate::generation::Gen;
use crate::paths::{Trust, read_text, resolve, shown};
use crate::state::AppState;

/// **Private.** The `override:` value of an edit, leaving out what is at its default.
fn override_value(o: &Override) -> Value {
    let mut v = json!({ "status": o.status });
    let m = v.as_object_mut().expect("an object");
    if !o.body.is_null() {
        m.insert("body".into(), o.body.clone());
    }
    if !o.headers.is_empty() {
        m.insert("headers".into(), json!(o.headers));
    }
    if let Some(c) = &o.content_type {
        m.insert("content_type".into(), json!(c));
    }
    if o.encoding.is_some() {
        m.insert("encoding".into(), json!(o.encoding));
    }
    v
}

/// **Private.** The patch that puts `value` at the route's `override:` in the endpoints file text,
/// in the file's own spelling of the key. `Err` is a message for the caller.
fn ops_for(text: &str, key: &str, value: &Value) -> Result<Vec<Op>, String> {
    let entries =
        parse_endpoints(text).map_err(|_| "the endpoints file is not valid".to_string())?;
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(text)
        .map_err(|_| "the endpoints file is not valid YAML".to_string())?;
    let map = match &doc {
        serde_yaml_ng::Value::Mapping(m) => Some(m),
        serde_yaml_ng::Value::Null => None,
        _ => return Err("the endpoints file is not a map".into()),
    };
    let spelled = entries
        .keys()
        .find(|k| endpoint_key(k).is_ok_and(|n| n == key))
        .cloned();
    let op = match (spelled, map) {
        (Some(k), Some(m)) => {
            let entry = m.get(serde_yaml_ng::Value::String(k.clone()));
            match entry {
                Some(serde_yaml_ng::Value::Mapping(e)) => {
                    let has = e.contains_key(serde_yaml_ng::Value::String("override".into()));
                    json!({ "op": if has { "set" } else { "insert" }, "path": [k, "override"], "value": value })
                }
                _ => json!({ "op": "set", "path": [k], "value": { "override": value } }),
            }
        }
        (None, Some(_)) => {
            json!({ "op": "insert", "path": [key], "value": { "override": value } })
        }
        (_, None) => {
            return Err("the endpoints file has no entries to add to; add one in the file".into());
        }
    };
    serde_json::from_value(json!([op])).map_err(|e| e.to_string())
}

/// `POST /__kaanal/payload/stage-file` — under the control lock, writes the edit (`{ route, status?, body?, headers?,
/// content_type?, encoding? }`, the body of `PUT /payload`) into the endpoints file; `{ file }`.
///
/// # Errors
/// 400 malformed body; 404 unknown route; 409 when the draft's endpoints are in `kaanal.yaml` (stage
/// through the draft), the file changed while it was written, or an Apply swapped the system;
/// 413 over the cap; 422 with `field` for a refused value or route.
pub async fn stage_file(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    req: Request,
) -> Result<Json<Value>, Reject> {
    let bytes = read_capped(req, Override::MAX_BODY + 4096)
        .await
        .map_err(|e| match e {
            ReadError::TooLarge => reject(StatusCode::PAYLOAD_TOO_LARGE, "body is too large"),
            ReadError::Broken => reject(StatusCode::BAD_REQUEST, "the body could not be read"),
        })?;
    let b: PutBody = serde_json::from_slice(&bytes)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let ov = b.to_override();
    ov.check()
        .map_err(|m| refused(crate::payload::PayloadError::Invalid(m)))?;
    let value = override_value(&ov);
    // The control lock, as `put` takes it: an Apply swaps the system under the same lock, so the
    // route is checked, the draft read and the file written against one system. The write is one
    // small synchronous file operation (at most a draft's size), so nothing awaits under the lock.
    let _c = lock_gen(&st, &g)?;
    let key = crate::payload::check_route(&g, &b.route).map_err(refused)?;
    let draft_text = st
        .draft
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .text
        .clone();
    let label = write_file(&draft_text, &st.dir, &key, &value)?;
    st.live
        .mark("payload", &format!("{} staged into {label}", b.route));
    Ok(Json(json!({ "file": label })))
}

/// **Private.** Reads the endpoints file, patches it, writes it back. The file name shown.
fn write_file(
    draft_text: &str,
    dir: &std::path::Path,
    key: &str,
    value: &Value,
) -> Result<String, Reject> {
    let cfg = Config::from_yaml(draft_text).map_err(|e| {
        reject(
            StatusCode::CONFLICT,
            &format!("the draft does not parse: {e}"),
        )
    })?;
    let Some(EndpointsConfig::File(p)) = cfg.endpoints else {
        return Err(reject(
            StatusCode::CONFLICT,
            "the draft's endpoints are in kaanal.yaml: stage the edit into the draft instead",
        ));
    };
    let path = resolve(dir, &p, Trust::Api, &keys(&["endpoints"]))
        .map_err(|e| reject(StatusCode::CONFLICT, &e.msg))?;
    let label = shown(dir, &path, Trust::Api);
    let src = read_text(&path).map_err(|e| {
        reject(
            StatusCode::CONFLICT,
            &format!("the endpoints file cannot be read: {e}"),
        )
    })?;
    if src.len() > crate::draft::MAX_DRAFT_BYTES {
        return Err(reject(
            StatusCode::CONFLICT,
            "the endpoints file is too large",
        ));
    }
    let ops = ops_for(&src, key, value).map_err(|m| reject(StatusCode::CONFLICT, &m))?;
    let done = patch::apply(&src, &ops).map_err(|m| reject(StatusCode::CONFLICT, &m))?;
    // the result must still be an endpoints file, or nothing is written
    parse_endpoints(&done.text).map_err(|_| {
        reject(
            StatusCode::CONFLICT,
            "the edit would leave the endpoints file invalid; nothing was written",
        )
    })?;
    // another writer between our read and our write: refuse rather than overwrite it
    if read_text(&path).ok().as_deref() != Some(src.as_str()) {
        return Err(reject(
            StatusCode::CONFLICT,
            "the endpoints file changed while it was being written; try again",
        ));
    }
    commit(&[Change::Write(path, done.text)], |a, b| {
        std::fs::rename(a, b)
    })
    .map_err(|e| {
        reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write failed: {e}"),
        )
    })?;
    Ok(label)
}
