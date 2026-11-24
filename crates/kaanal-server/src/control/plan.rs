//! The sizing planner route: `POST /sim/plan` (Step 9c-1, RULES 141).
//!
//! A plan walks the system in force toward a design that holds a target load (`kaanal_sim::plan`),
//! adds the database-family what-ifs, writes the proposed draft into the applied text and re-checks
//! it (`sim::plan_draft`), then sweeps it once for the measured knee. It is a run record of kind
//! `plan` under the same single slot as a run and a sweep (`409` with the holder's id), `202` and a
//! `Location` to poll, its steps added to the record as they are taken. As a sweep: the applied
//! design only (no `source`), its first profile when none is named, the live chaos unless a
//! `scenario` is named, and the record carries the seed, the fingerprint and the chaos digest.
//! Kaanal never applies the plan: the draft and the design in force are left as they were.

use super::sim::{Applied, accepted, applied_inputs, busy, from_sim, parse_profile};
use super::sweep::{chaos_digest, limit, live_chaos};
use super::*;
use crate::sim::plan_draft::{
    FAMILY_MAX_STEPS, FamilySection, LABEL, PlanAsked, PlanDoc, TargetAsked, judge_budget,
    proposed_text, round_trip, same_design, verify,
};
use crate::sim::plan_search::{Cheapest, cheapest_asked, run_cheapest};
use crate::sim::profile::{Profile, RunRequest, first_profile, to_sim};
use crate::sim::run::{Job, Slot, execute_sweep};
use crate::sim::store::{RunRecord, SweepMeta};
use kaanal_core::NodeKind;
use kaanal_core::resize::alternatives;
use kaanal_sim::plan::{CHEAPEST_NOTE, FamilyAnswer, PlanVerdict, Request, Step, Target, walk};
use kaanal_sim::sweep::Options;
use kaanal_sim::{Limits, MAX_RATE_PER_S, SimConfig};

/// Lowest and highest `target.multiple`: 64 is the sweep's top factor.
const MULTIPLE: std::ops::RangeInclusive<f64> = 0.1..=64.0;
/// Most `max_steps` a request may ask for (default 30, spec).
const MAX_STEPS: u32 = 100;
/// What each database-family answer says (spec: a what-if, not a recommendation to migrate).
const FAMILY_NOTE: &str = "assumes your application can use this family; preset estimates";

/// The accepted body of `POST /__kaanal/sim/plan`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanBody {
    /// A profile name from `traffic:`, or a profile written out (default the applied design's first).
    #[serde(default)]
    profile: Option<Value>,
    /// Seed of the draws at every candidate (default the config's `seed:`).
    #[serde(default)]
    seed: Option<u64>,
    /// Plan under this scenario's chaos alone, instead of the live chaos.
    #[serde(default)]
    scenario: Option<String>,
    /// The load to hold: `{ multiple }` or `{ req_s }`.
    target: TargetAsked,
    /// Highest p99 that passes, ms (default 1000).
    #[serde(default)]
    p99_ms: Option<f64>,
    /// Highest error rate that passes, percent (default 1).
    #[serde(default)]
    error_pct: Option<f64>,
    /// The most the plan may cost, USD an hour of the priced nodes.
    #[serde(default)]
    budget_per_hour: Option<f64>,
    /// Nodes the plan must not move.
    #[serde(default)]
    keep: Vec<String>,
    /// The most steps, up to 100 (default 30).
    #[serde(default)]
    max_steps: Option<u32>,
    /// Search for the cheapest design that holds, with provider sections (Step 9c-2; default false).
    #[serde(default)]
    cheapest: bool,
    /// The whole time limit of a cheapest plan, seconds (1 to 600, default 150).
    #[serde(default)]
    time_limit_s: Option<u32>,
    /// The most runs per search (1 to 2000, default 200).
    #[serde(default)]
    run_cap: Option<u32>,
    /// The providers to compare (`aws`, `gcp`, `azure`, or `["none"]`).
    #[serde(default)]
    providers: Option<Vec<String>>,
}

fn bad(m: &str) -> Reject {
    reject(StatusCode::BAD_REQUEST, m)
}

/// **Private.** The target, each bound checked against `base`'s streams (the factor and the offered
/// rate).
fn target(t: &TargetAsked, base: &SimConfig) -> Result<(Target, f64, f64), Reject> {
    let target = match (t.multiple, t.req_s) {
        (Some(m), None) if MULTIPLE.contains(&m) => Target::Multiple(m),
        (Some(_), None) => return Err(bad("target.multiple must be 0.1 to 64")),
        (None, Some(r)) if r.is_finite() && r > 0.0 => Target::PerSecond(r),
        (None, Some(_)) => return Err(bad("target.req_s must be a rate above 0")),
        _ => return Err(bad("target must be { multiple } or { req_s }, one of them")),
    };
    let k = target.factor(base).map_err(|e| bad(&e))?;
    let top = base
        .streams
        .iter()
        .map(|s| s.rate_per_s * k)
        .fold(0.0, f64::max);
    if top > MAX_RATE_PER_S {
        return Err(bad(&format!(
            "the target puts a stream at {top:.0} req/s, over the engine's cap of {MAX_RATE_PER_S} req/s"
        )));
    }
    let req_s = base.streams.iter().map(|s| s.rate_per_s * k).sum();
    Ok((target, k, req_s))
}

/// **Private.** The budget in micro-USD an hour.
fn budget(b: Option<f64>) -> Result<Option<u64>, Reject> {
    b.map(|usd| {
        if usd.is_finite() && usd >= 0.0 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked finite and not negative; saturates far past any hourly price"
            )]
            Ok((usd * 1e6).round() as u64)
        } else {
            Err(bad("budget_per_hour must be dollars an hour, 0 or more"))
        }
    })
    .transpose()
}

/// **Private.** What must hold before a plan takes the slot, cheapest first: the slot is free (409
/// with the holder's id), a CPU-work permit is free (429, as a draft run: the next check builds the
/// applied text), there is an applied text (409), and it builds to the design in force `g` (409
/// [`crate::sim::plan_draft::DIFFERS`]). The applied text and the design in force can disagree for a
/// moment (an Apply installs its generation before it writes `applied`) or until the next Apply (a
/// live edit whose structure waits for one, a file the loader read changed on disk); the job checks
/// again for an Apply in between. Returns the applied text.
///
/// # Errors
/// As above; 500 when the build task died.
async fn ready_to_start(st: &Arc<AppState>, g: &Arc<Generation>) -> Result<String, Reject> {
    if let Some(running) = st.sims.running() {
        return Err(busy(&running));
    }
    let permit = Arc::clone(&st.work).try_acquire_owned().map_err(|_| {
        reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many drafts being built at once; retry",
        )
    })?;
    let applied = st
        .draft
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .applied
        .clone();
    if applied.is_empty() {
        return Err(reject(
            StatusCode::CONFLICT,
            "no applied text to propose a draft from: the server was not started from a kaanal.yaml",
        ));
    }
    let (st2, text, g) = (Arc::clone(st), applied.clone(), Arc::clone(g));
    tokio::task::spawn_blocking(move || {
        let _held = permit;
        same_design(&st2, &text, &g)
    })
    .await
    .map_err(|e| reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    .map_err(|e| reject(StatusCode::CONFLICT, &e))?;
    Ok(applied)
}

/// `POST /__kaanal/sim/plan` — starts a sizing plan of the system in force.
///
/// # Errors
/// 400 for a bad body, target, limit, budget, step cap or `keep` name, or no `profile` and none
/// defined; 404 for an unknown profile or scenario; 409 when a run, sweep or plan is in progress or
/// there is no system.
pub async fn post_plan(State(st): State<Arc<AppState>>, body: Bytes) -> Result<Response, Reject> {
    let b: PlanBody = serde_json::from_slice(&body).map_err(|e| bad(&format!("bad body: {e}")))?;
    let lim = limit(b.p99_ms, b.error_pct)?;
    let budget_micro_usd_hr = budget(b.budget_per_hour)?;
    let max_steps = b.max_steps.unwrap_or(30);
    if !(1..=MAX_STEPS).contains(&max_steps) {
        return Err(bad(&format!("max_steps must be 1 to {MAX_STEPS}")));
    }
    let posted = b.profile.as_ref().filter(|v| v.is_object()).cloned();
    let parsed = b.profile.map(parse_profile).transpose()?;
    let mut req = RunRequest {
        profile: Profile::Named(String::new()),
        duration: None,
        warmup: None,
        seed: b.seed,
        scenario: b.scenario,
    };
    let Applied {
        g,
        chaos,
        snap,
        runtime,
    } = applied_inputs(&st, &req)?;
    let cfg = &g.config;
    if let Some(k) = b.keep.iter().find(|k| !cfg.nodes.contains_key(*k)) {
        return Err(bad(&format!(
            "keep: '{k}' is not a node of the design in force"
        )));
    }
    let (secs, cap, provs) = (b.time_limit_s, b.run_cap, b.providers);
    let cheapest = cheapest_asked(b.cheapest, secs, cap, provs, cfg).map_err(|e| bad(&e))?;
    let (label, inline_profile) = if let Some((label, profile)) = parsed {
        let inline = match profile {
            Profile::Inline(_) => posted,
            Profile::Named(_) => None,
        };
        req.profile = profile;
        (label, inline)
    } else {
        let name = first_profile(&g).ok_or_else(|| {
            bad("no traffic profile to plan for: the applied design has no traffic: section; add one, or send a profile")
        })?;
        req.profile = Profile::Named(name.to_owned());
        (name.to_owned(), None)
    };
    let mut planned = to_sim(&g, &req).map_err(|e| from_sim(&e))?;
    let (tgt, k, req_s) = target(&b.target, &planned.cfg)?;
    let asked = PlanAsked {
        target: b.target,
        factor: k,
        req_s,
        limit: lim,
        budget_micro_usd_hr,
        keep: b.keep,
        max_steps,
        cheapest,
    };
    let applied = ready_to_start(&st, &g).await?;
    let id = st
        .sims
        .begin_plan(
            g.id,
            &label,
            req.scenario.as_deref(),
            SweepMeta {
                seed: b.seed.unwrap_or(g.seed),
                max_factor: None,
                fingerprint: g.fingerprint.clone(),
                inline_profile,
                live_chaos: snap.map(|e| live_chaos(&g, &e)),
                chaos_digest: chaos_digest(&g, &chaos),
                runtime_scenarios: runtime,
            },
        )
        .map_err(|running| busy(&running))?;
    let slot = Slot::new(Arc::clone(&st), id.clone());
    let id2 = id.clone();
    tokio::task::spawn_blocking(move || {
        planned.fill_sizes(&g);
        let job = Job {
            g,
            chaos,
            cfg: planned.cfg,
        };
        let r = Request {
            target: tgt,
            limit: asked.limit.clone(),
            budget_micro_usd_hr: asked.budget_micro_usd_hr,
            keep: asked.keep.clone(),
            max_steps: asked.max_steps,
        };
        let result = run_plan(&st, &id2, &job, &r, &applied, asked);
        slot.finish_plan(result);
    });
    Ok(accepted(&id))
}

/// **Private.** The plan job: the walk (its steps to the record as taken), the family sections, the
/// proposed draft, its re-check and one sweep of it.
///
/// # Errors
/// An applied text that is no longer the design in force (an Apply ran since the route checked), a
/// walk that cannot run, and the internal failures (spec anti-goal: the draft names the design
/// that was checked): a proposed draft that does
/// not patch or build, or a re-check of it that is not exactly the walk's end, whatever the verdict.
fn run_plan(
    st: &AppState,
    id: &str,
    job: &Job,
    req: &Request,
    applied: &str,
    asked: PlanAsked,
) -> Result<(PlanDoc, String), String> {
    let (g, k) = (&job.g, asked.factor);
    let internal = |e: String| format!("internal: {e}");
    // again: an Apply may have run since the route checked (not internal: the message says retry)
    same_design(st, applied, g)?;
    let on_step = &mut |s: &Step| st.sims.plan_step(id, s.clone());
    // a cheapest plan (Step 9c-2) replaces the walk and the family what-ifs with the search and the
    // provider sections; the tail below is the same for both
    let (plan, families, search, providers) = match &asked.cheapest {
        None => {
            let plan = walk(
                &g.config,
                &g.endpoints,
                &job.chaos,
                &job.cfg,
                req,
                &Limits::server(),
                on_step,
            )?;
            (plan, families(job, req), None, None)
        }
        Some(c) => {
            let Cheapest { main, providers } = run_cheapest(job, req, c, k, on_step)?;
            (main.plan, Vec::new(), Some(main.info), Some(providers))
        }
    };
    let text = proposed_text(applied, &plan.steps)
        .map_err(|e| internal(format!("the proposed draft cannot be written: {e}")))?;
    let (verify, proposed) =
        verify(st, &text, g, &job.chaos, &job.cfg, k, &req.limit).map_err(internal)?;
    round_trip(&plan.end, &verify).map_err(internal)?;
    let claims = matches!(
        plan.verdict,
        PlanVerdict::Holds
            | PlanVerdict::AlreadyHolds
            | PlanVerdict::OverBudget { holds: true, .. }
    );
    if claims && !verify.holds {
        return Err(internal(format!(
            "the proposed draft does not hold the target when checked again (p99 {:.1} ms, errors {:.2} percent), so no plan is given",
            verify.p99_ms,
            verify.error_rate * 100.0
        )));
    }
    let opts = Options {
        limit: req.limit.clone(),
        max_factor: (2.0 * k).clamp(1.0, *MULTIPLE.end()),
        ..Options::default()
    };
    let swept = execute_sweep(
        &Job {
            g: proposed,
            chaos: job.chaos.clone(),
            cfg: job.cfg.clone(),
        },
        &opts,
        &mut |_| {},
    );
    let (measured_knee, sweep_error) = match swept {
        Ok(s) => (Some(s.verdict), None),
        Err(e) => (None, Some(e)),
    };
    // spec: the budget counts per-use prices at the target run's visits, which the walk leaves out
    let (verdict, budget_check) = judge_budget(
        plan.verdict,
        asked.budget_micro_usd_hr,
        verify.cost.total_micro_usd_hr,
    );
    let mut doc = PlanDoc {
        label: LABEL,
        request: asked,
        verdict,
        budget_check,
        steps: plan.steps,
        start: plan.start,
        end: plan.end,
        price_before: plan.price_before,
        price_after: plan.price_after,
        unpriced_nodes: plan.unpriced_nodes,
        families,
        verify,
        measured_knee,
        sweep_error,
        cheapest_note: search.is_some().then_some(CHEAPEST_NOTE),
        search,
        providers,
        comparison: None,
    };
    doc.comparison = crate::sim::report::compare(&doc);
    Ok((doc, text))
}

/// **Private.** The database-family section: for each database, the walk on each flavor of the other
/// family (`neo4j` left out, spec); a database or a flavor with nothing to walk keeps its reason.
fn families(job: &Job, req: &Request) -> Vec<FamilySection> {
    let (g, mut out) = (&job.g, Vec::new());
    // each alternative's walk is capped, so several databases cannot hold the slot for minutes
    let req = &Request {
        max_steps: req.max_steps.min(FAMILY_MAX_STEPS),
        ..req.clone()
    };
    let skip = |node: &str, flavor: Option<&str>, reason: String| FamilySection::Skipped {
        node: node.to_string(),
        flavor: flavor.map(str::to_string),
        reason,
    };
    for (node, n) in &g.config.nodes {
        if n.kind() != NodeKind::Database {
            continue;
        }
        let alts = match alternatives(&g.config, node, &req.keep, false) {
            Ok(a) => a,
            Err(why) => {
                out.push(skip(node, None, why.to_string()));
                continue;
            }
        };
        for a in alts.kept {
            let flavor = a.flavor.as_str();
            let walked = walk(
                &a.config,
                &g.endpoints,
                &job.chaos,
                &job.cfg,
                req,
                &Limits::server(),
                &mut |_| {},
            );
            out.push(match walked {
                Ok(plan) => {
                    let mut notes = a.notes;
                    notes.push(FAMILY_NOTE.to_string());
                    FamilySection::Answer(FamilyAnswer {
                        node: node.clone(),
                        flavor: flavor.to_string(),
                        plan: Box::new(plan),
                        notes,
                    })
                }
                Err(e) => skip(node, Some(flavor), e),
            });
        }
        for (f, why) in alts.dropped {
            out.push(skip(node, Some(f.as_str()), why));
        }
    }
    out
}

/// `GET /__kaanal/sim/runs/{id}/draft` — a finished plan's proposed draft: the applied text with
/// the plan's steps patched in (`text/plain; charset=utf-8`), the text the plan was checked on.
///
/// # Errors
/// 404 for an unknown id; 409 for a record that is not a plan, a plan still running or one that
/// failed (it has no draft; the body says why).
pub async fn get_plan_draft(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, Reject> {
    let rec = st.sims.get(&id).ok_or_else(|| {
        reject(
            StatusCode::NOT_FOUND,
            "no such run (only the last eight are kept)",
        )
    })?;
    proposed_draft_of(rec)
        .map(|text| ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response())
}

/// **Private.** The proposed draft of record `rec`.
///
/// # Errors
/// 409 for a record that is not a plan, a plan still running or one that failed; 500 for a plan that
/// is done without its draft (a server bug, as the report route says of a missing field).
fn proposed_draft_of(rec: RunRecord) -> Result<String, Reject> {
    let conflict = |m: String| reject(StatusCode::CONFLICT, &m);
    if rec.kind != "plan" {
        return Err(conflict(format!("a {} has no proposed draft", rec.kind)));
    }
    match (rec.state, rec.proposed_draft) {
        ("running", _) => Err(conflict("the plan is not finished".into())),
        ("failed", _) => Err(conflict(format!(
            "the plan failed and has no proposed draft: {}",
            rec.error.unwrap_or_default()
        ))),
        (_, Some(text)) => Ok(text),
        (_, None) => Err(reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the plan record has no proposed_draft",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::store::{LiveChaos, SimStore, SweepMeta};

    fn meta() -> SweepMeta {
        SweepMeta {
            seed: 1,
            max_factor: None,
            fingerprint: "0123456789ab".into(),
            inline_profile: None,
            live_chaos: Some(LiveChaos {
                scenarios: Vec::new(),
                entries: 0,
                digest: "e3b0c44298fc".into(),
            }),
            chaos_digest: "9f86d081884c".into(),
            runtime_scenarios: Vec::new(),
        }
    }

    fn status(r: Result<String, Reject>) -> StatusCode {
        r.unwrap_err().0
    }

    #[test]
    fn the_draft_of_a_plan_by_state() {
        let s = SimStore::default();
        let id = s.begin_plan(1, "peak", None, meta()).unwrap();
        // running: not finished (deterministic: the record is held in that state, no race)
        let running = s.get(&id).unwrap();
        assert_eq!(
            status(proposed_draft_of(running.clone())),
            StatusCode::CONFLICT
        );
        // done with its draft: the text
        let done = RunRecord {
            state: "done",
            proposed_draft: Some("version: 1\n".into()),
            ..running.clone()
        };
        assert_eq!(proposed_draft_of(done).unwrap(), "version: 1\n");
        // done without one is a server bug, not "failed"
        let broken = RunRecord {
            state: "done",
            ..running.clone()
        };
        assert_eq!(
            status(proposed_draft_of(broken)),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        // failed: 409 with the reason
        s.finish_plan(&id, Err("internal: boom".into()));
        let err = proposed_draft_of(s.get(&id).unwrap()).unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(
            err.1.0.to_string().contains("internal: boom"),
            "{:?}",
            err.1
        );
        // not a plan
        let other = RunRecord {
            kind: "sweep",
            ..running
        };
        assert_eq!(status(proposed_draft_of(other)), StatusCode::CONFLICT);
    }

    fn state() -> Arc<AppState> {
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\ntraffic:\n  peak: { rps: 10, journeys: [ { endpoint: 'GET /users/{id}' } ] }\n";
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let cfg = kaanal_core::Config::from_yaml(yaml).unwrap();
        Arc::new(AppState::build(&cfg, &dir, String::new(), 0).unwrap())
    }

    const PLAIN: &str = r#"{ "target": { "multiple": 2 } }"#;
    const CHEAPEST: &str = r#"{ "target": { "multiple": 2 }, "cheapest": true, "run_cap": 5 }"#;

    async fn post_status(st: &Arc<AppState>, body: &'static str) -> StatusCode {
        match post_plan(State(Arc::clone(st)), Bytes::from(body)).await {
            Ok(r) => r.status(),
            Err((code, _)) => code,
        }
    }

    /// The route builds the applied text, so it takes a CPU-work permit as a draft run does: 429
    /// while every permit is held, before any build.
    #[tokio::test]
    async fn every_work_permit_held_is_429() {
        let st = state();
        let held = Arc::clone(&st.work)
            .try_acquire_many_owned(u32::try_from(crate::sim::WORK_PERMITS).unwrap())
            .unwrap();
        assert_eq!(post_status(&st, PLAIN).await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            post_status(&st, CHEAPEST).await,
            StatusCode::TOO_MANY_REQUESTS,
            "a cheapest plan keeps the order"
        );
        drop(held);
        assert_ne!(post_status(&st, PLAIN).await, StatusCode::TOO_MANY_REQUESTS);
    }

    /// Busy comes first: a plan while the slot is held is 409, even with no permit free.
    #[tokio::test]
    async fn busy_comes_before_the_permit() {
        let st = state();
        let _held = Arc::clone(&st.work)
            .try_acquire_many_owned(u32::try_from(crate::sim::WORK_PERMITS).unwrap())
            .unwrap();
        let _run = st.sims.begin(1, "peak", None, "applied", None).unwrap();
        assert_eq!(post_status(&st, PLAIN).await, StatusCode::CONFLICT);
        assert_eq!(post_status(&st, CHEAPEST).await, StatusCode::CONFLICT);
    }
}
