//! The draft routes: `GET` and `PUT /__kaanal/draft`.

use super::*;
use crate::draft::MAX_DRAFT_BYTES;
use crate::draft::apply::{self as apply_mod, ApplyError};
use crate::draft::layout::Layout;
use crate::draft::patch::{self, Op};
use crate::draft::rename;
use crate::draft::save::{self as draft_save, Plan, SaveError};

/// **Private.** The draft as the API shows it: `{ text, dirty, generation, lost, rev, doc,
/// doc_error, project_id, project_name, kind_defaults, kinds, boot, layout, layout_note, layout_dirty, saved, renames }` (`layout_dirty`: the
/// positions differ from the sidecar, so a Save would write them) (`generation` is 0 before the first Apply of a blank
/// start; `doc` is the text as JSON, or null with `doc_error` saying why).
fn view(st: &AppState) -> Json<Value> {
    // copy what the view needs and let go of the lock before parsing, so a big draft does not
    // hold up PUT and Apply
    let (
        text,
        dirty,
        generation,
        lost,
        rev,
        file,
        mut layout,
        layout_note,
        layout_dirty,
        saved,
        renames,
        cached,
    ) = {
        let d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
        (
            d.text.clone(),
            d.dirty(),
            st.generation().map_or(0, |g| g.id),
            d.lost.clone(),
            d.rev,
            d.file.clone(),
            d.layout.clone(),
            d.layout_note.clone(),
            d.layout_dirty(),
            d.saved(),
            d.renames.clone(),
            d.doc_cache
                .as_ref()
                .filter(|(r, _)| *r == d.rev)
                .map(|(_, p)| Arc::clone(p)),
        )
    };
    // parsed once per `rev`: Studio polls this route, and a parse is the dear part
    let parsed = cached.unwrap_or_else(|| {
        let p = Arc::new(doc::draft_doc(&text));
        let mut d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
        // only if the text is still the one parsed (a PUT may have landed meanwhile)
        if d.rev == rev {
            d.doc_cache = Some((rev, Arc::clone(&p)));
        }
        p
    });
    let (doc, doc_error) = (&parsed.0, &parsed.1);
    // positions of nodes the text does not define are not shown (only when the text is a config)
    if let Some(nodes) = doc
        .as_ref()
        .and_then(|d| d.get("nodes"))
        .and_then(Value::as_object)
    {
        layout.0.retain(|k, _| nodes.contains_key(k));
    }
    Json(json!({
        "text": text, "dirty": dirty, "generation": generation, "lost": lost, "rev": rev,
        "boot": doc::boot_id(),
        "doc": doc, "doc_error": doc_error,
        "project_id": doc::project_id(file.as_deref()),
        "project_name": doc::project_name(file.as_deref()), "kind_defaults": doc::kind_defaults(),
        "kinds": doc::kinds(),
        "layout": layout, "layout_note": layout_note, "layout_dirty": layout_dirty,
        "saved": saved, "renames": renames,
    }))
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

/// The accepted body of `PUT /__kaanal/draft`: the whole new `text`, or a `patch` of edits, and/or
/// a `layout` (node name to `[x, y]`, replacing the whole map).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutBody {
    text: Option<String>,
    patch: Option<Vec<Op>>,
    layout: Option<Layout>,
}

/// `PUT /__kaanal/draft` — replaces the draft text, or applies a `patch` to it (`[{ op: set |
/// insert | remove, path, value }]`, kept to the bytes it touches so comments and style survive),
/// and/or replaces the `layout`, and returns the new view. What a patch could not keep is in
/// `lost`; a `rename_node` moves the node's position too. The text is not validated (a half-typed
/// file must be savable); `POST /draft/validate` does that. A layout change never bumps `rev`.
///
/// # Errors
/// 400 for a body with both `text` and `patch` or none of `text`, `patch` and `layout`, a text
/// over 1 MiB (1048576 bytes), a patch that cannot be made (nothing at the path, a key already
/// there, a draft that is not YAML), or a layout that cannot be kept (over 2000 positions, an
/// empty name, a coordinate beyond ±1e6, over 256 KiB as a file); the draft is unchanged.
pub async fn put_draft(
    State(st): State<Arc<AppState>>,
    payload: Result<Json<PutBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    if b.text.is_some() && b.patch.is_some() {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "send either `text` or `patch`, not both",
        ));
    }
    if b.text.is_none() && b.patch.is_none() && b.layout.is_none() {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            "send `text` or `patch`, and/or `layout`",
        ));
    }
    if let Some(l) = &b.layout {
        l.check().map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    }
    let (rev_before, edited, extra) = {
        let mut d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
        let rev_before = d.rev;
        let mut extra = None;
        match (b.text, b.patch) {
            (Some(t), None) if t.len() > MAX_DRAFT_BYTES => return Err(too_large(t.len())),
            (Some(t), None) => {
                if d.text != t {
                    d.rev += 1;
                }
                d.text = t;
                d.lost.clear();
                d.renames.clear();
            }
            (None, Some(ops)) => {
                let out =
                    patch::apply(&d.text, &ops).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
                if out.text.len() > MAX_DRAFT_BYTES {
                    return Err(too_large(out.text.len()));
                }
                if d.text != out.text {
                    d.rev += 1;
                }
                d.text = out.text;
                d.lost.extend(out.lost);
                for (from, to) in &out.renamed {
                    d.layout.rename(from, to);
                }
                d.renames.extend(out.renamed);
                extra = Some((out.touched, out.notes));
            }
            // a layout alone (both was refused above)
            _ => {}
        }
        if let Some(l) = b.layout {
            d.layout = l;
        }
        (rev_before, d.rev, extra)
    };
    // a page is watching the live run: a parameter edit goes straight into the engine (step 8L)
    let live = if edited == rev_before {
        None // the text did not change (a layout alone, or the same text again)
    } else {
        Some(crate::live::edit::apply_params(&st).await)
    };
    let mut v = view(&st);
    if let Some(outcome) = live {
        v.0["live"] = outcome.to_json();
    }
    if let Some((touched, notes)) = extra {
        // where a rename_node changed the text, and what it could not reach
        v.0["touched"] = json!(touched);
        v.0["notes"] = json!(notes);
    }
    Ok(v)
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
    /// Point them at the renamed node, for the `rename_node` operations made since the draft was
    /// set (400 when there were none).
    Rewrite,
}

/// The accepted body of `POST /__kaanal/draft/save`.
#[derive(Deserialize, Default)]
pub struct SaveBody {
    #[serde(default)]
    confirm_loss: bool,
    #[serde(default)]
    fix_scenarios: Fix,
    /// Write even though the file on disk is not what Studio read (the answer to a conflict).
    #[serde(default)]
    overwrite: bool,
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

/// The JSON body of a control request that takes optional options: `None` when the body is empty (or
/// only whitespace), whatever the content type says, which axum's `Json` would refuse; a body that is
/// not a JSON `T` is a 400. The request carries no content type check: Studio always sends JSON.
fn json_opt<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<Option<T>, Reject> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    serde_json::from_slice(body)
        .map(Some)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("invalid JSON body: {e}")))
}

/// `POST /__kaanal/draft/save` — writes the draft text to the config file (atomically) and
/// answers `{ ok, written, removed }`. The running system is not touched; that is Apply. With
/// `fix_scenarios: "remove"` the scenario files the saved text no longer resolves against are
/// deleted in the same Save (every file changes or none). After the config, the layout goes to
/// `kaanal.layout.json` (`layout_written`, and `layout_note` when it was not: a link or non-file
/// there is refused, and the config write stands).
///
/// # Errors
/// 409 with `lost` when patches dropped something and `confirm_loss` is not true; 409 with
/// `conflict: true` when the file on disk is not what Studio read (nothing is written); 409 when
/// the server has no config file; 400 for a draft that names a file outside the project (as in
/// validate and Apply), for `fix_scenarios: "rewrite"` (not available yet) or `"remove"` on a
/// draft that does not build; 500 when a write fails (every file is put back).
pub async fn save_draft(
    State(st): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let body = json_opt::<SaveBody>(&body)?.unwrap_or_default();
    let (confirm, fix, overwrite) = (body.confirm_loss, body.fix_scenarios, body.overwrite);
    let (dir, seed, key, port) = (st.dir.clone(), st.drawn_seed, st.auth_key.clone(), st.port);
    let st2 = st.clone();
    let done = tokio::task::spawn_blocking(move || {
        draft_save::save(&st2.draft, &st2.save_lock, confirm, overwrite, |text, renames| {
            // Save never writes a draft that names files outside the project
            crate::paths::confine(text, &dir).map_err(|e| e.msg)?;
            if fix == Fix::None {
                return Ok(Plan::default());
            }
            if fix == Fix::Rewrite && renames.is_empty() {
                return Err("fix_scenarios: rewrite needs a rename_node made since the draft was set; there is none (use remove or none)".into());
            }
            let flagged = crate::draft::apply::build_draft(text, &dir, seed, key, port)
                .map(|b| stale_scenarios(&b.generation.flagged, &dir))
                .map_err(|d| {
                    format!(
                        "the draft does not build, so its scenario files cannot be checked: {}",
                        d.message
                    )
                })?;
            if fix == Fix::Remove {
                return Ok(Plan {
                    remove: flagged,
                    ..Plan::default()
                });
            }
            // rewrite: a file that names none of the renamed nodes stays flagged; one that cannot
            // be rewritten safely stops the whole Save
            let mut rewrite = Vec::new();
            for path in flagged {
                let old = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())))?;
                match rename::rewrite_scenario(&old, renames) {
                    Ok(Some(new)) => rewrite.push((path, new)),
                    Ok(None) => {}
                    Err(m) => {
                        let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        return Err(format!("cannot rewrite scenarios/{file}, so nothing was saved: {m}"));
                    }
                }
            }
            Ok(Plan {
                rewrite,
                ..Plan::default()
            })
        })
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    match done {
        Ok(s) => Ok(Json(json!({
            "ok": true, "written": s.written, "removed": s.removed,
            "layout_written": s.layout_written, "layout_note": s.layout_note,
        }))),
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

/// `POST /__kaanal/draft/reload` — throws the draft away and reads the config file again (and the
/// layout sidecar beside it): the answer to a Save conflict when the file on disk is the one to
/// keep. The draft is then clean against the disk (`saved`), whatever was applied stays applied
/// (`dirty` says whether it differs), and `rev` moves if the text did. Returns the new view.
///
/// # Errors
/// 409 when the server has no config file or the file cannot be read (it is missing, say); 400 for
/// a file over 1 MiB. The draft is unchanged.
pub async fn reload_draft(State(st): State<Arc<AppState>>) -> Result<Json<Value>, Reject> {
    let file = st
        .draft
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .file
        .clone()
        .ok_or_else(|| {
            reject(
                StatusCode::CONFLICT,
                "this server was not started from a config file; there is nothing to reload",
            )
        })?;
    // a config that is a link is read through, as Save writes through it
    let target = file.canonicalize().unwrap_or_else(|_| file.clone());
    let (text, layout) = tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&target).map_err(|e| e.to_string())?;
        Ok::<_, String>((text, file.parent().map(crate::draft::layout::load)))
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    .map_err(|m| {
        reject(
            StatusCode::CONFLICT,
            &format!("cannot read the config file: {m}"),
        )
    })?;
    if text.len() > MAX_DRAFT_BYTES {
        return Err(too_large(text.len()));
    }
    {
        let mut d = st.draft.lock().unwrap_or_else(PoisonError::into_inner);
        if d.text != text {
            d.rev += 1;
        }
        d.text.clone_from(&text);
        d.base = Some(text);
        d.lost.clear();
        d.renames.clear();
        if let Some((l, note)) = layout {
            d.layout = l.clone();
            d.saved_layout = l;
            d.layout_note = note;
        }
    }
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
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let text = match json_opt::<DraftBody>(&body)? {
        Some(b) if b.text.len() > MAX_DRAFT_BYTES => return Err(too_large(b.text.len())),
        Some(b) => b.text,
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
        crate::draft::apply::build_draft(&text, &dir, seed, key, port).map(|b| {
            let g = &b.generation;
            (g.warnings.clone(), g.flagged.clone(), route_owners(g))
        })
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(Json(match built {
        Ok((warnings, flagged, owners)) => json!({
            "ok": true, "diagnostics": [], "warnings": warnings, "flagged": flagged,
            "owners": owners,
        }),
        Err(d) => json!({
            "ok": false, "diagnostics": [d], "warnings": [], "flagged": [], "owners": {},
        }),
    }))
}

/// **Private.** Which component owns each route of the built draft, by the route's normalized
/// key (B8: the Studio hosts a route's Steps on its owner, because a chain is what the owner
/// calls). Static routes (no owner) and synthesized `HEAD` copies are left out.
fn route_owners(g: &crate::generation::Generation) -> serde_json::Map<String, Value> {
    g.endpoints
        .iter()
        .filter(|e| !e.synthesized)
        .filter_map(|e| Some((e.key.clone(), json!(g.topo.nodes[e.owner?].name))))
        .collect()
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
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let opts = json_opt::<ApplyBody>(&body)?.unwrap_or_default();
    let ms = opts.timeout_ms.unwrap_or(DEFAULT_DRAIN_MS);
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
            "payloads_dropped": a.payloads_dropped,
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
