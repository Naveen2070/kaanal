//! The load sweep route: `POST /sim/sweep` (Step 9a).
//!
//! A sweep runs the system in force at a ladder of load factors and finds the knee
//! ([`kaanal_sim::sweep`]). It is a run record of kind `sweep`: the same single slot as a run
//! (`409` with the holder's id), `202` and a `Location` to poll, the points added to the record
//! as they finish. There is no `source` (the draft is not swept) and no `duration_s`/`warmup_s`
//! (each point sizes its own window), so a body naming them is a `400` (unknown field).

use super::sim::{accepted, applied_inputs, busy, from_sim, parse_profile};
use super::*;
use crate::sim::profile::{Profile, RunRequest, to_sim};
use crate::sim::report::{ReportError, sweep_report};
use crate::sim::run::{Job, Slot, execute_sweep};
use crate::sim::store::SweepMeta;
use kaanal_sim::sweep::{Limit, Options};

/// Highest `max_factor` the route takes: 64x the profile's rate is far past any knee worth
/// finding, and the ladder grows by sqrt(2) a step, so 64 is 17 points at most before refinement (up to 23 runs with it).
const MAX_FACTOR: f64 = 64.0;

/// The accepted body of `POST /__kaanal/sim/sweep`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepBody {
    /// A profile name from `traffic:`, or a profile written out.
    profile: Value,
    /// Seed of the draws at every point (default the config's `seed:`).
    #[serde(default)]
    seed: Option<u64>,
    /// Sweep under this scenario's chaos alone, instead of the live chaos.
    #[serde(default)]
    scenario: Option<String>,
    /// Highest p99 that passes, in milliseconds: above 0 (default 1000).
    #[serde(default)]
    p99_ms: Option<f64>,
    /// Highest error rate that passes, in percent: 0 to 100 (default 1). The record carries it as
    /// a fraction, `limit.error_rate`.
    #[serde(default)]
    error_pct: Option<f64>,
    /// Highest load factor on the ladder: 1 to 64 (default 8).
    #[serde(default)]
    max_factor: Option<f64>,
}

/// **Private.** The sweep options from a body, each limit checked.
fn options(b: &SweepBody) -> Result<Options, Reject> {
    let bad = |m: &str| reject(StatusCode::BAD_REQUEST, m);
    // a zero Duration also catches a p99 too small to be a nanosecond
    let p99 = Duration::try_from_secs_f64(b.p99_ms.unwrap_or(1000.0) / 1000.0)
        .ok()
        .filter(|d| !d.is_zero())
        .ok_or_else(|| bad("p99_ms must be a number of milliseconds above 0"))?;
    let error_pct = b.error_pct.unwrap_or(1.0);
    if !(0.0..=100.0).contains(&error_pct) {
        return Err(bad("error_pct must be a percentage, 0 to 100"));
    }
    let max_factor = b.max_factor.unwrap_or(8.0);
    if !(1.0..=MAX_FACTOR).contains(&max_factor) {
        return Err(bad(&format!("max_factor must be 1 to {MAX_FACTOR}")));
    }
    Ok(Options {
        limit: Limit {
            p99,
            error_rate: error_pct / 100.0,
        },
        max_factor,
        ..Options::default()
    })
}

/// `POST /__kaanal/sim/sweep` — starts a load sweep of the system in force.
///
/// # Errors
/// 400 for a bad body or limit, 404 for an unknown profile or scenario, 409 when a run or sweep
/// is in progress or there is no system.
pub async fn post_sweep(State(st): State<Arc<AppState>>, body: Bytes) -> Result<Response, Reject> {
    let b: SweepBody = serde_json::from_slice(&body)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let opts = options(&b)?;
    // only an object can be an inline profile: keep a copy for the record, not for a name
    let posted = b.profile.is_object().then(|| b.profile.clone());
    let (label, profile) = parse_profile(b.profile)?;
    let inline_profile = match profile {
        Profile::Inline(_) => posted,
        Profile::Named(_) => None,
    };
    let req = RunRequest {
        profile,
        duration: None,
        warmup: None,
        seed: b.seed,
        scenario: b.scenario,
    };
    let (g, chaos) = applied_inputs(&st, &req)?;
    let mut planned = to_sim(&g, &req).map_err(|e| from_sim(&e))?;
    // no `check`: the sweep sizes each point itself and holds every one to the server caps
    let id = st
        .sims
        .begin_sweep(
            g.id,
            &label,
            req.scenario.as_deref(),
            SweepMeta {
                seed: b.seed.unwrap_or(g.seed),
                max_factor: opts.max_factor,
                fingerprint: g.fingerprint.clone(),
                inline_profile,
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
        let result = execute_sweep(&job, &opts, &mut |p| st.sims.sweep_point(&id2, p.clone()));
        slot.finish_sweep(result);
    });
    Ok(accepted(&id))
}

/// The query of `GET /__kaanal/sim/runs/{id}/report`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportQuery {
    /// `markdown` (default) or `json`.
    format: Option<String>,
}

/// `GET /__kaanal/sim/runs/{id}/report` — a finished sweep's report: the Markdown
/// (`text/markdown; charset=utf-8`), or with `?format=json` its data alone.
///
/// # Errors
/// 400 for a bad `format`; 404 for an unknown id; 409 while the sweep runs, for a failed sweep
/// (the body carries why) and for a plain run; 500 for a sweep record missing its recipe.
pub async fn get_sweep_report(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    q: Result<Query<ReportQuery>, QueryRejection>,
) -> Result<Response, Reject> {
    let Query(q) = q.map_err(|e| reject(StatusCode::BAD_REQUEST, &e.body_text()))?;
    let json_form = match q.format.as_deref() {
        None | Some("markdown") => false,
        Some("json") => true,
        Some(_) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "format must be markdown or json",
            ));
        }
    };
    let rec = st.sims.get(&id).ok_or_else(|| {
        reject(
            StatusCode::NOT_FOUND,
            "no such run (only the last eight are kept)",
        )
    })?;
    let rep = sweep_report(&rec, env!("CARGO_PKG_VERSION")).map_err(|e| match e {
        ReportError::NotASweep => reject(
            StatusCode::CONFLICT,
            "run reports are made in Studio; only a sweep has a report here",
        ),
        ReportError::Running => reject(StatusCode::CONFLICT, "the sweep is not finished"),
        ReportError::Failed(why) => reject(
            StatusCode::CONFLICT,
            &format!("the sweep failed and has no report: {why}"),
        ),
        ReportError::Internal(field) => reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("the sweep record has no {field}"),
        ),
    })?;
    Ok(if json_form {
        Json(rep.json).into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            rep.markdown,
        )
            .into_response()
    })
}
