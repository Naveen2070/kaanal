//! A parameter edit goes straight into the running engine (step 8L, RULES 105 s). After the draft
//! changed, the running config (kept in the session) is compared with the draft:
//!
//! - when the draft has the same components and wires, the draft is the new running config;
//! - when it has pending structure edits (a new or removed component or wire, a changed kind),
//!   the new running config is the running structure with the draft's parameters for every
//!   component and wire that exists in both, and the structure stays in the draft for Apply.
//!
//! The new system is built as an Apply builds it, but the engine is kept: the generation is
//! swapped without a drain and the engine is reconfigured in place (`Engine::reconfigure`), so
//! queues and running work carry over. Chaos, scenarios and pins are remapped as an Apply does.
//! The file on disk changes only on Save, and the draft text is never rewritten here.

use std::sync::Arc;

use kaanal_core::Config;
use serde_json::{Value, json};

use crate::draft::apply::build_config;
use crate::draft::remap::remap_control;
use crate::state::AppState;

/// What a draft change did to the running system.
#[derive(Debug, PartialEq)]
pub(crate) enum EditOutcome {
    /// No page is connected, so nothing runs live: the draft waits for Apply as before.
    NoSession,
    /// The draft changes no parameter of the running system (it may still hold structure edits).
    Unchanged { structure_pending: bool },
    /// These components and wires now run with the new parameters.
    Applied {
        changed: Vec<String>,
        structure_pending: bool,
    },
    /// The change could not be applied; the running system is as it was.
    Invalid(String),
}

impl EditOutcome {
    /// The part of the draft's reply that tells the page what happened.
    pub(crate) fn to_json(&self) -> Value {
        match self {
            EditOutcome::NoSession => json!({ "live": "none" }),
            EditOutcome::Unchanged { structure_pending } => {
                json!({ "live": "unchanged", "structure_pending": structure_pending })
            }
            EditOutcome::Applied {
                changed,
                structure_pending,
            } => json!({
                "live": "applied",
                "changed": changed,
                "structure_pending": structure_pending,
            }),
            EditOutcome::Invalid(m) => json!({ "live": "invalid", "message": m }),
        }
    }
}

/// The new running config for `draft` over `running`, the names of what changed in it, and whether
/// the draft holds structure edits that wait for Apply.
fn hybrid(running: &Config, draft: &Config) -> (Config, Vec<String>, bool) {
    let mut h = running.clone();
    let mut changed = Vec::new();
    let mut pending = h.nodes.len() != draft.nodes.len() || h.edges.len() != draft.edges.len();
    for (name, d) in &draft.nodes {
        match h.nodes.get_mut(name) {
            Some(r) if std::mem::discriminant(r) == std::mem::discriminant(d) => {
                if format!("{r:?}") != format!("{d:?}") {
                    changed.push(name.clone());
                }
                *r = d.clone();
            }
            _ => pending = true,
        }
    }
    let mut used = vec![false; draft.edges.len()];
    for r in &mut h.edges {
        let hit = draft
            .edges
            .iter()
            .enumerate()
            .find(|(j, d)| !used[*j] && d.from == r.from && d.to == r.to);
        if let Some((j, d)) = hit {
            used[j] = true;
            if format!("{r:?}") != format!("{d:?}") {
                changed.push(format!("{} -> {}", r.from, r.to));
            }
            *r = d.clone();
        } else {
            pending = true;
        }
    }
    pending |= used.iter().any(|u| !u);
    if !pending {
        // the same structure: everything else (traffic, endpoints, locale, ...) is the draft's too
        let top_changed = format!("{running:?}") != format!("{draft:?}") && changed.is_empty();
        h = draft.clone();
        if top_changed {
            changed.push("the file".into());
        }
    }
    (h, changed, pending)
}

/// Applies the draft's parameter changes to the running system, if a live session runs.
pub(crate) async fn apply_params(st: &Arc<AppState>) -> EditOutcome {
    if st.live.running().is_none() {
        return EditOutcome::NoSession;
    }
    let _one_at_a_time = st.apply_lock.lock().await;
    // read again: another edit or an Apply may have changed it while this one waited
    let Some(running) = st.live.running() else {
        return EditOutcome::NoSession;
    };
    let (text, dir, seed, key, port) = {
        let d = st
            .draft
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            d.text.clone(),
            st.dir.clone(),
            st.drawn_seed,
            st.auth_key.clone(),
            st.port,
        )
    };
    let draft = match Config::from_yaml(&text) {
        Ok(d) => d,
        // the reason a half-typed or refused draft gives (a position and what is wrong)
        Err(e) => return EditOutcome::Invalid(e.to_string()),
    };
    let (next, changed, structure_pending) = hybrid(&running, &draft);
    if changed.is_empty() {
        return EditOutcome::Unchanged { structure_pending };
    }
    let built = {
        let (cfg, text) = (next.clone(), text.clone());
        tokio::task::spawn_blocking(move || build_config(&cfg, &text, &dir, seed, key, port)).await
    };
    let built = match built {
        Ok(Ok(b)) => b,
        Ok(Err(d)) => return EditOutcome::Invalid(d.message),
        Err(e) => return EditOutcome::Invalid(e.to_string()),
    };
    let (mut generation, scenarios) = (built.generation, built.scenarios);
    let (Some(old), Some(engine)) = (
        st.generation(),
        st.generation().and_then(|g| g.engine.clone()),
    ) else {
        return EditOutcome::NoSession;
    };
    generation.id = old.id;
    // a parameter edit is not a restart: the system keeps its clock
    generation.started = old.started;
    generation.engine = Some(engine.clone());
    let topo = Arc::new(generation.topo.clone());
    {
        let mut c = crate::control::lock(st);
        let backup = (*c).clone();
        if let Err(m) = remap_control(&old, &generation, scenarios, &mut c) {
            return EditOutcome::Invalid(m);
        }
        if let Err(m) = st.publish(&c) {
            *c = backup;
            return EditOutcome::Invalid(m);
        }
        st.install(generation);
    }
    if let Err(m) = engine.reconfigure(topo).await {
        return EditOutcome::Invalid(m);
    }
    st.live.set_running(next);
    if !structure_pending {
        // the engine now runs exactly what the draft says, so nothing waits for Apply (unless the
        // draft moved on while this was building: then the next edit decides)
        let mut d = st
            .draft
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if d.text == text {
            d.applied = text;
        }
    }
    let mut names = changed
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if changed.len() > 3 {
        names.push_str(", ...");
    }
    st.live.mark("param", &names);
    EditOutcome::Applied {
        changed,
        structure_pending,
    }
}
