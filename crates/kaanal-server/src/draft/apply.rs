//! Turning draft text into a system: the build off to the side and its diagnostics. Validate
//! uses it now; Apply will build the same way before it swaps.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::diagnostics::{Located, PathSeg};
use kaanal_core::{Config, ConfigError, ScenarioSet};
use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::{AuthKey, Issuer};
use crate::draft::remap::Report;
use crate::generation::{Flagged, Generation, Trust};
use crate::state::{AppState, BusEvent, Control};

/// One problem in a draft, placed in its text. The loader stops at the first error, so a
/// validation returns one at most.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Diagnostic {
    /// The file the problem is in, when it is not `kaanal.yaml` (a spec or the endpoints file).
    pub file: Option<String>,
    /// 1-based line in the draft; `None` when the error has no place (or is in another file).
    pub line: Option<usize>,
    /// Column in that line, 0-based (R1.15).
    pub col: Option<usize>,
    /// The path into the config (`["edges", 1, "to"]`); empty for a syntax error.
    pub path: Vec<Value>,
    /// What is wrong.
    pub message: String,
}

impl Diagnostic {
    /// A parse error of the draft itself.
    pub(crate) fn parsed(e: ConfigError) -> Diagnostic {
        Diagnostic {
            file: None,
            line: e.line,
            col: e.col,
            path: Vec::new(),
            message: e.msg,
        }
    }

    /// A load error, placed in `text` when it is in `kaanal.yaml`.
    fn located(e: &Located, text: &str, dir: &Path) -> Diagnostic {
        let path = e
            .path
            .iter()
            .map(|s| match s {
                PathSeg::Key(k) => json!(k),
                PathSeg::Index(i) => json!(i),
            })
            .collect();
        // an error in another file has its own position (or none); `text` is not that file
        let (line, col) = match &e.file {
            None => {
                let at = e.in_source(text);
                (at.line, at.col)
            }
            Some(_) => e.pos.unzip(),
        };
        Diagnostic {
            file: e.file.as_ref().map(|f| relative(dir, f)),
            line,
            col,
            path,
            message: scrub(&e.msg, dir),
        }
    }
}

/// What a draft builds: the generation (issuer built, no engine yet) and the scenarios resolved
/// against its topology.
pub(crate) struct Built {
    pub(crate) generation: Generation,
    pub(crate) scenarios: ScenarioSet,
}

/// `file` as a client may see it: relative to the project folder (the last two components when
/// it is outside it, like a scenario file under its canonical folder), never an absolute path.
pub(crate) fn relative(dir: &Path, file: &Path) -> String {
    let canonical = dir.canonicalize().ok();
    for d in std::iter::once(dir).chain(canonical.as_deref()) {
        if let Ok(rel) = file.strip_prefix(d) {
            return rel.display().to_string();
        }
    }
    let tail: Vec<_> = file.components().rev().take(2).collect();
    tail.iter()
        .rev()
        .collect::<std::path::PathBuf>()
        .display()
        .to_string()
}

/// `msg` with the project folder (as given and as the loader's canonical form) written as `.`, so a
/// message never shows where the project lives on the machine.
fn scrub(msg: &str, dir: &Path) -> String {
    let mut out = msg.to_string();
    if let Ok(c) = dir.canonicalize() {
        let c = c.display().to_string();
        // Windows canonical paths start with `\\?\`; messages may carry either form
        let plain = c.strip_prefix(r"\\?\").unwrap_or(&c).to_string();
        out = out.replace(&c, ".").replace(&plain, ".");
    }
    let here = dir.display().to_string();
    if here != "." {
        out = out.replace(&here, ".");
    }
    out
}

/// Builds the system `text` describes, without starting anything: parse, then the same load
/// pipeline `serve` and `check` run. `dir` resolves spec and data files; `drawn_seed` is the
/// server's startup draw, so a realistic config keeps its seed. The auth issuer is built here
/// too (from `auth_key` and `port`), so a draft with an auth node and no key fails in validate
/// as it does in Apply.
///
/// # Errors
/// The first problem, as a [`Diagnostic`].
pub(crate) fn build_draft(
    text: &str,
    dir: &Path,
    drawn_seed: Option<u64>,
    auth_key: Option<AuthKey>,
    port: u16,
) -> Result<Built, Diagnostic> {
    let cfg = Config::from_yaml(text).map_err(Diagnostic::parsed)?;
    build_config(&cfg, text, dir, drawn_seed, auth_key, port)
}

/// [`build_draft`] for a config already parsed (`text` is what it came from, so a problem can be
/// placed in it). A live parameter edit builds the running structure with the draft's parameters
/// this way, which has no text of its own.
///
/// # Errors
/// The first problem, as a [`Diagnostic`].
pub(crate) fn build_config(
    cfg: &Config,
    text: &str,
    dir: &Path,
    drawn_seed: Option<u64>,
    auth_key: Option<AuthKey>,
    port: u16,
) -> Result<Built, Diagnostic> {
    // the draft decides, not the startup config: `controlled` runs on its `seed:`, `realistic`
    // on the seed drawn at startup (or one drawn now when startup was controlled)
    let drawn_seed = match cfg.seed_source() {
        kaanal_core::SeedSource::Fixed(_) => None,
        kaanal_core::SeedSource::Entropy => Some(drawn_seed.unwrap_or_else(rand::random)),
    };
    let (mut generation, scenarios) = Generation::build(cfg, dir, drawn_seed, Trust::Api)
        .map_err(|e| Diagnostic::located(&e, text, dir))?;
    generation.auth =
        Issuer::from_config(cfg, auth_key, port).map_err(|e| Diagnostic::located(&e, text, dir))?;
    // what goes back to a client names files inside the project, never the machine's layout
    for w in &mut generation.warnings {
        *w = scrub(w, dir);
    }
    for f in &mut generation.flagged {
        f.file = relative(dir, &f.path);
        f.message = scrub(&f.message, dir);
    }
    Ok(Built {
        generation,
        scenarios,
    })
}

/// Why an Apply did not happen. Nothing changed in any of these cases.
pub(crate) enum ApplyError {
    /// The draft does not build.
    Invalid(Diagnostic),
    /// Requests in flight did not finish inside the budget; the old system keeps running.
    Timeout,
    /// The build task died, or the new chaos could not be published.
    Internal(String),
}

/// What an Apply did.
pub(crate) struct Applied {
    /// The generation now in force.
    pub(crate) generation: u64,
    /// The live chaos entries and active scenarios the swap kept, by what they are.
    pub(crate) kept: Value,
    /// What the swap dropped, by name (see `kept`).
    pub(crate) dropped: Value,
    /// Scenario files left out because they name something the new system lacks.
    pub(crate) flagged: Vec<Flagged>,
    /// Routes whose live payload edit the swap dropped (the route is gone or takes none now).
    pub(crate) payloads_dropped: Vec<String>,
}

/// Applies the draft: builds the new system off to the side, holds mock traffic back, waits up
/// to `timeout` for the requests in flight to finish, swaps, and lets traffic go on. On a drain
/// timeout the old system keeps running and nothing else changes.
///
/// Order, and the locks it takes: `apply_lock` (one Apply at a time), then the gate (paused for
/// the drain), then the `control` lock for the swap itself: chaos, scenarios and the generation
/// change together under it, so a control request never pairs the old topology with the new
/// chaos. Counters start again at zero with the new generation.
///
/// Chaos, runtime scenarios and pins are kept where what they name still exists, by name (see
/// [`super::remap`]); the rest is dropped and reported. Scenario files that no longer resolve
/// are left out and flagged. Collections and route counters start over. An Apply always
/// rebuilds from disk, so it also picks up edited spec, data and scenario files, even when the
/// draft text did not change.
pub(crate) async fn apply(st: &Arc<AppState>, timeout: Duration) -> Result<Applied, ApplyError> {
    apply_from(st, timeout, false).await
}

/// Rebuilds the **running** system from the applied text, not the draft: a fresh engine with
/// empty queues and a fresh live timeline, the draft untouched (the live Reset). Chaos, scenarios
/// and pins are kept as in an Apply. The caller checks that something has been applied.
pub(crate) async fn reset(st: &Arc<AppState>, timeout: Duration) -> Result<Applied, ApplyError> {
    apply_from(st, timeout, true).await
}

async fn apply_from(
    st: &Arc<AppState>,
    timeout: Duration,
    from_applied: bool,
) -> Result<Applied, ApplyError> {
    let _one_at_a_time = st.apply_lock.lock().await;
    let text = {
        let d = st
            .draft
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if from_applied {
            d.applied.clone()
        } else {
            d.text.clone()
        }
    };
    let (dir, seed) = (st.dir.clone(), st.drawn_seed);
    let (key, port, source) = (st.auth_key.clone(), st.port, text.clone());
    // reads spec and data files, so off the async threads
    let built = tokio::task::spawn_blocking(move || build_draft(&source, &dir, seed, key, port))
        .await
        .map_err(|e| ApplyError::Internal(e.to_string()))?
        .map_err(ApplyError::Invalid)?;
    let Built {
        mut generation,
        scenarios,
    } = built;
    generation.id = st.generation().map_or(1, |g| g.id + 1);
    generation.engine = Some(crate::actor::spawn(kaanal_core::replay::Engine::live(
        &generation.topo,
    )));

    let _live_hold = st.live.hold_for_apply();
    let _paused = st.gate.pause();
    if !st.gate.drained(timeout).await {
        return Err(ApplyError::Timeout);
    }
    let id = generation.id;
    let flagged = generation.flagged.clone();
    let (kept, dropped, payloads_dropped) = {
        let mut c = crate::control::lock(st);
        let backup = (*c).clone();
        // None only for a blank start, which has no chaos to keep
        let report = if let Some(old) = st.generation() {
            super::remap::remap_control(&old, &generation, scenarios, &mut c)
                .map_err(ApplyError::Internal)?
        } else {
            *c = Control {
                scenarios,
                ..Control::default()
            };
            Report::default()
        };
        if let Err(m) = st.publish(&c) {
            // the old generation stays, so the old control state must too
            *c = backup;
            return Err(ApplyError::Internal(format!(
                "publishing the new chaos: {m}"
            )));
        }
        // Collections belong to the system that defined them: an item of a removed or changed
        // `collection:` must not leak into the new one. Like the route counters, they start over.
        st.collections.clear();
        // a live edit belongs to a route of the system it was made on (step 8A)
        let payloads_dropped = st.prune_payloads(&generation);
        st.install(generation);
        (report.kept, report.dropped, payloads_dropped)
    };
    // the applied text first: the restarted live session reads it as the config it runs
    st.draft
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .applied = text;
    // a running live session restarts on the new engine, with a fresh timeline (RULES 105 n)
    st.live.restart(st);
    let _ = st.events.send(BusEvent::Applied { generation: id });
    Ok(Applied {
        generation: id,
        kept,
        dropped,
        flagged,
        payloads_dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::CollectionOp;

    #[test]
    fn messages_and_files_never_show_the_project_folder_in_either_spelling() {
        let tmp = std::env::temp_dir().join(format!("kaanal-scrub-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        // the same folder spelled with a `..`: the loader's canonical form differs from what was given
        let given = tmp.join("sub").join("..");
        let canonical = given.canonicalize().unwrap();
        let msg = format!(
            "cannot read {} and {}",
            canonical.join("a.yaml").display(),
            given.join("b.yaml").display()
        );
        let shown = scrub(&msg, &given);
        assert!(!shown.contains(&tmp.display().to_string()), "{shown}");
        assert!(
            shown.contains("a.yaml") && shown.contains("b.yaml"),
            "{shown}"
        );
        assert_eq!(
            relative(&given, &canonical.join("scenarios").join("f.yaml")),
            format!("scenarios{}f.yaml", std::path::MAIN_SEPARATOR)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    const YAML: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n";

    #[tokio::test]
    async fn an_apply_starts_the_collections_over() {
        let cfg = Config::from_yaml(YAML).unwrap();
        let st = Arc::new(AppState::build(&cfg, Path::new("."), "t".into(), 0).unwrap());
        st.draft.lock().unwrap().text = YAML.to_string();
        let add = |n: &str| {
            st.collections
                .apply(n, CollectionOp::Add, None, Some(serde_json::json!({})))
        };
        assert_eq!(add("notes").0, 201);
        let list = |n: &str| st.collections.apply(n, CollectionOp::List, None, None).1;
        assert_eq!(list("notes"), serde_json::json!([{ "id": 1 }]));
        assert!(apply(&st, Duration::from_secs(1)).await.is_ok());
        assert_eq!(
            list("notes"),
            serde_json::json!([]),
            "the new system starts with none"
        );
    }
}
