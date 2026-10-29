//! A simulation of the draft text, without Apply: the draft is built into a throwaway
//! [`Generation`] that never reaches `AppState`, with the live chaos carried onto it by name.

use std::sync::Arc;
use std::time::Duration;

use kaanal_core::ChaosStack;
use kaanal_sim::SimConfig;

use super::profile::RunRequest;
use super::run::Job;
use crate::draft::apply::{Built, Diagnostic, build_draft};
use crate::state::{AppState, Control};

/// Why a draft run could not start. Nothing was taken (no run slot) in any of these cases.
pub(crate) enum DraftRunError {
    /// The draft does not build.
    Invalid(Diagnostic),
    /// The request names a scenario the draft's scenarios (with the live runtime ones) lack.
    UnknownScenario(String),
    /// Every CPU-work permit is taken; retry.
    Busy,
    /// The build task died, or the chaos could not be carried over.
    Internal(String),
}

/// Builds the current draft and the chaos for it: the live chaos remapped onto the draft's
/// topology by name (what the draft lacks is dropped), or one named scenario when `req` has one.
/// Returns the job (its `cfg` is a placeholder the caller replaces from the profile) and the
/// draft's `rev`. Nothing is stored or published: the live control state is only read.
///
/// # Errors
/// See [`DraftRunError`].
pub(crate) async fn draft_job(
    st: &Arc<AppState>,
    req: &RunRequest,
) -> Result<(Job, u64), DraftRunError> {
    let permit = Arc::clone(&st.work)
        .try_acquire_owned()
        .map_err(|_| DraftRunError::Busy)?;
    let (text, rev) = {
        let d = st
            .draft
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (d.text.clone(), d.rev)
    };
    let (st2, scenario) = (Arc::clone(st), req.scenario.clone());
    // reads spec and data files, so off the async threads; the permit lives as long as the work
    let job = tokio::task::spawn_blocking(move || {
        let _held = permit;
        let Built {
            mut generation,
            scenarios,
        } = build_draft(
            &text,
            &st2.dir,
            st2.drawn_seed,
            st2.auth_key.clone(),
            st2.port,
        )
        .map_err(DraftRunError::Invalid)?;
        // the record's `generation` is the one in force; the run's `draft_rev` says what it was
        // the control state and the generation as one pair: an Apply swaps both under this lock,
        // so the scratch copy's node ids always belong to `old`
        let (mut scratch, old) = {
            let c = crate::control::lock(&st2);
            (c.clone(), st2.generation())
        };
        generation.id = old.as_ref().map_or(0, |g| g.id);
        match old {
            Some(old) => {
                crate::draft::remap::remap_control(&old, &generation, scenarios, &mut scratch)
                    .map_err(DraftRunError::Internal)?;
            }
            // a blank start has no live chaos to carry
            None => {
                scratch = Control {
                    scenarios,
                    ..Control::default()
                };
            }
        }
        let chaos = if let Some(name) = scenario {
            let mut stack = ChaosStack::default();
            stack
                .set_active(&scratch.scenarios, std::slice::from_ref(&name))
                .map_err(|_| DraftRunError::UnknownScenario(name))?;
            stack.compile(&scratch.scenarios)
        } else {
            scratch.stack.compile(&scratch.scenarios)
        };
        Ok(Job {
            g: Arc::new(generation),
            chaos,
            cfg: SimConfig {
                streams: Vec::new(),
                duration: Duration::ZERO,
                warmup: Duration::ZERO,
                seed: 0,
            },
        })
    })
    .await
    .map_err(|e| DraftRunError::Internal(e.to_string()))??;
    Ok((job, rev))
}
