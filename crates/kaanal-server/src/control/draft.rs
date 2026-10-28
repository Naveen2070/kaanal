//! The draft routes: `GET` and `PUT /__kaanal/draft`.

use super::*;
use crate::draft::MAX_DRAFT_BYTES;
use crate::draft::apply::{self as apply_mod, ApplyError};
use crate::draft::patch::{self, Op};
use crate::draft::save::{self as draft_save, SaveError};

/// **Private.** The draft as the API shows it: `{ text, dirty, generation }` (`generation` is 0
/// before the first Apply of a blank start).
fn view(st: &AppState) -> Json<Value> {
    let d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
    let generation = st.generation().map_or(0, |g| g.id);
    Json(json!({ "text": d.text, "dirty": d.dirty(), "generation": generation, "lost": d.lost }))
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

/// The accepted body of `PUT /__kaanal/draft`: the whole new `text`, or a `patch` of edits.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutBody {
    text: Option<String>,
    patch: Option<Vec<Op>>,
}

/// `PUT /__kaanal/draft` — replaces the draft text, or applies a `patch` to it (`[{ op: set |
/// insert | remove, path, value }]`, kept to the bytes it touches so comments and style survive),
/// and returns the new view. What a patch could not keep is in `lost`. The text is not validated
/// (a half-typed file must be savable); `POST /draft/validate` does that.
///
/// # Errors
/// 400 for a body with both or neither of `text` and `patch`, a text over 1 MiB (1048576 bytes),
/// or a patch that cannot be made (nothing at the path, a key already there, a draft that is not
/// YAML); the draft is unchanged.
pub async fn put_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Json<PutBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let mut d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
    match (b.text, b.patch) {
        (Some(t), None) if t.len() > MAX_DRAFT_BYTES => return Err(too_large(t.len())),
        (Some(t), None) => {
            d.text = t;
            d.lost.clear();
        }
        (None, Some(ops)) => {
            let out =
                patch::apply(&d.text, &ops).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
            if out.text.len() > MAX_DRAFT_BYTES {
                return Err(too_large(out.text.len()));
            }
            d.text = out.text;
            d.lost.extend(out.lost);
        }
        _ => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "send either `text` or `patch`",
            ));
        }
    }
    drop(d);
    Ok(view(&st))
}

/// What Save does with the scenario files the draft no longer resolves against (the `flagged`
/// ones of validate and Apply).
#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Fix {
    /// Leave them (the default); the next `kaanal up` fails on them until they are fixed.
    #[default]
    None,
    /// Delete the files, so the next `kaanal up` loads.
    Remove,
    /// Point them at the renamed node; arrives with rename.
    Rewrite,
}

/// The accepted body of `POST /__kaanal/draft/save`.
#[derive(Deserialize, Default)]
pub struct SaveBody {
    #[serde(default)]
    confirm_loss: bool,
    #[serde(default)]
    fix_scenarios: Fix,
}

/// **Private.** The flagged scenario files Save may delete: regular files (not links) directly in
/// the project's own `scenarios/` folder (not a link to somewhere else), as the loader found
/// them. Anything else stays where it is.
fn stale_scenarios(
    flagged: &[crate::generation::Flagged],
    dir: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    let folder = dir.join("scenarios");
    let own = std::fs::symlink_metadata(&folder).is_ok_and(|m| m.is_dir());
    let canonical = folder.canonicalize().ok();
    let mut out: Vec<std::path::PathBuf> = flagged
        .iter()
        .filter(|f| {
            own && f.path.parent().and_then(|p| p.canonicalize().ok()) == canonical
                && std::fs::symlink_metadata(&f.path).is_ok_and(|m| m.is_file())
        })
        .map(|f| f.path.clone())
        .collect();
    out.dedup();
    out
}

/// `POST /__kaanal/draft/save` — writes the draft text to the config file (atomically) and
/// answers `{ ok, written, removed }`. The running system is not touched; that is Apply. With
/// `fix_scenarios: "remove"` the scenario files the saved text no longer resolves against are
/// deleted in the same Save (every file changes or none).
///
/// # Errors
/// 409 with `lost` when patches dropped something and `confirm_loss` is not true; 409 with
/// `conflict: true` when the file on disk is not what Studio read (nothing is written); 409 when
/// the server has no config file; 400 for a draft that names a file outside the project (as in
/// validate and Apply), for `fix_scenarios: "rewrite"` (not available yet) or `"remove"` on a
/// draft that does not build; 500 when a write fails (every file is put back).
pub async fn save_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Option<Json<SaveBody>>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let body = payload
        .map_err(|e| reject(e.status(), &e.body_text()))?
        .map_or_else(SaveBody::default, |Json(b)| b);
    if body.fix_scenarios == Fix::Rewrite {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "fix_scenarios: rewrite needs a rename, which is not available yet; use remove or none",
        ));
    }
    let (confirm, fix) = (body.confirm_loss, body.fix_scenarios);
    let (dir, seed, key, port) = (st.dir.clone(), st.drawn_seed, st.auth_key.clone(), st.port);
    let st2 = st.clone();
    let done = tokio::task::spawn_blocking(move || {
        draft_save::save(&st2.draft, &st2.save_lock, confirm, |text| {
            // Save never writes a draft that names files outside the project
            crate::generation::confine(text, &dir).map_err(|e| e.msg)?;
            if fix != Fix::Remove {
                return Ok(Vec::new());
            }
            crate::draft::apply::build_draft(text, &dir, seed, key, port)
                .map(|b| stale_scenarios(&b.generation.flagged, &dir))
                .map_err(|d| {
                    format!(
                        "the draft does not build, so its scenario files cannot be checked: {}",
                        d.message
                    )
                })
        })
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    match done {
        Ok(s) => Ok(Json(
            json!({ "ok": true, "written": s.written, "removed": s.removed }),
        )),
        Err(SaveError::Lost(lost)) => Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("the draft lost: {}; send confirm_loss to save anyway", lost.join("; ")),
                "lost": lost,
            })),
        )),
        Err(SaveError::Conflict) => Err((
            StatusCode::CONFLICT,
            Json(
                json!({ "error": "the file changed on disk since Studio read it; nothing was written", "conflict": true }),
            ),
        )),
        Err(SaveError::NoFile) => Err(reject(
            StatusCode::CONFLICT,
            "this server was not started from a config file; there is nothing to save to",
        )),
        Err(SaveError::Plan(m)) => Err(reject(StatusCode::BAD_REQUEST, &m)),
        Err(SaveError::Io(m)) => Err(reject(StatusCode::INTERNAL_SERVER_ERROR, &m)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::Flagged;

    fn flagged(path: std::path::PathBuf) -> Flagged {
        Flagged {
            file: "scenarios/f.yaml".into(),
            scenario: "f".into(),
            message: String::new(),
            path,
        }
    }

    #[test]
    fn only_regular_files_in_the_projects_own_scenarios_folder_may_be_deleted() {
        let tmp = std::env::temp_dir().join(format!("kaanal-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (own, elsewhere) = (tmp.join("proj/scenarios"), tmp.join("other/scenarios"));
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let (mine, theirs) = (own.join("f.yaml"), elsewhere.join("f.yaml"));
        std::fs::write(&mine, "chaos: []").unwrap();
        std::fs::write(&theirs, "chaos: []").unwrap();
        let dir = tmp.join("proj");
        // a file the loader found elsewhere is not deleted, even though it is named scenarios/f.yaml
        assert_eq!(
            stale_scenarios(&[flagged(mine.clone()), flagged(theirs)], &dir),
            vec![mine.clone()]
        );
        // a folder is not a file
        std::fs::create_dir(own.join("d.yaml")).unwrap();
        let none = Vec::<std::path::PathBuf>::new();
        assert_eq!(stale_scenarios(&[flagged(own.join("d.yaml"))], &dir), none);
        // no scenarios folder at all
        assert_eq!(stale_scenarios(&[flagged(mine)], &tmp.join("other")), none);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
