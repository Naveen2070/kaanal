//! The load sweep route: `POST /sim/sweep` (Step 9a).
//!
//! A sweep runs the system in force at a ladder of load factors and finds the knee
//! ([`kaanal_sim::sweep`]). It is a run record of kind `sweep`: the same single slot as a run
//! (`409` with the holder's id), `202` and a `Location` to poll, the points added to the record
//! as they finish. With no `profile` it sweeps the applied design's first one
//! ([`first_profile`]). There is no `source` (the draft is not swept) and no `duration_s`/`warmup_s`
//! (each point sizes its own window), so a body naming them is a `400` (unknown field).

use super::sim::{Applied, accepted, applied_inputs, busy, from_sim, parse_profile};
use super::*;
use crate::sim::profile::{Profile, RunRequest, first_profile, to_sim};
use crate::sim::report::{ReportError, report};
use crate::sim::run::{Job, Slot, execute_sweep};
use crate::sim::store::{LiveChaos, SweepMeta};
use kaanal_core::fingerprint::{canonical_digest, combine};
use kaanal_sim::sweep::{Limit, Options};

/// Highest `max_factor` the route takes: 64x the profile's rate is far past any knee worth
/// finding, and the ladder grows by sqrt(2) a step, so 64 is 17 points at most before refinement (up to 23 runs with it).
const MAX_FACTOR: f64 = 64.0;

/// The accepted body of `POST /__kaanal/sim/sweep`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepBody {
    /// A profile name from `traffic:`, or a profile written out (default the applied design's
    /// first profile; Studio sends none, as the draft it shows may not be the design swept).
    #[serde(default)]
    profile: Option<Value>,
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

/// **Private.** The pass line from a body's `p99_ms` (default 1000) and `error_pct` (default 1),
/// each checked (the sweep and the plan share it).
pub(super) fn limit(p99_ms: Option<f64>, error_pct: Option<f64>) -> Result<Limit, Reject> {
    let bad = |m: &str| reject(StatusCode::BAD_REQUEST, m);
    // a zero Duration also catches a p99 too small to be a nanosecond
    let p99 = Duration::try_from_secs_f64(p99_ms.unwrap_or(1000.0) / 1000.0)
        .ok()
        .filter(|d| !d.is_zero())
        .ok_or_else(|| bad("p99_ms must be a number of milliseconds above 0"))?;
    let error_pct = error_pct.unwrap_or(1.0);
    if !(0.0..=100.0).contains(&error_pct) {
        return Err(bad("error_pct must be a percentage, 0 to 100"));
    }
    Ok(Limit {
        p99,
        error_rate: error_pct / 100.0,
    })
}

/// **Private.** The sweep options from a body, each limit checked.
fn options(b: &SweepBody) -> Result<Options, Reject> {
    let limit = limit(b.p99_ms, b.error_pct)?;
    let max_factor = b.max_factor.unwrap_or(8.0);
    if !(1.0..=MAX_FACTOR).contains(&max_factor) {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            &format!("max_factor must be 1 to {MAX_FACTOR}"),
        ));
    }
    Ok(Options {
        limit,
        max_factor,
        ..Options::default()
    })
}

/// `POST /__kaanal/sim/sweep` — starts a load sweep of the system in force.
///
/// # Errors
/// 400 for a bad body or limit, or no `profile` and none defined; 404 for an unknown profile or
/// scenario; 409 when a run or sweep is in progress or there is no system.
pub async fn post_sweep(State(st): State<Arc<AppState>>, body: Bytes) -> Result<Response, Reject> {
    let b: SweepBody = serde_json::from_slice(&body)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let opts = options(&b)?;
    // only an object can be an inline profile: keep a copy for the record, not for a name
    let posted = b.profile.as_ref().filter(|v| v.is_object()).cloned();
    let parsed = b.profile.map(parse_profile).transpose()?;
    let mut req = RunRequest {
        // set below, once the applied design is known
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
    let (label, inline_profile) = if let Some((label, profile)) = parsed {
        let inline = match profile {
            Profile::Inline(_) => posted,
            Profile::Named(_) => None,
        };
        req.profile = profile;
        (label, inline)
    } else {
        let name = first_profile(&g).ok_or_else(|| {
            reject(
                StatusCode::BAD_REQUEST,
                "no traffic profile to sweep: the applied design has no traffic: section; add one, or send a profile",
            )
        })?;
        req.profile = Profile::Named(name.to_owned());
        (name.to_owned(), None)
    };
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
                max_factor: Some(opts.max_factor),
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
        let result = execute_sweep(&job, &opts, &mut |p| st.sims.sweep_point(&id2, p.clone()));
        slot.finish_sweep(result);
    });
    Ok(accepted(&id))
}

/// **Private.** The live chaos of `snap` for a sweep record: the active scenarios (an ordered stack,
/// a later one wins, so kept in order), the live entry count, and a digest of the live entries. Each
/// entry is its `GET /chaos` view plus its key by name (no `expires_in_ms`), written as compact JSON
/// with sorted keys; the entries are a set, so their texts are sorted before hashing, and neither the
/// order they were set in nor the config's node order (ids) changes the digest. The digest is the
/// fingerprint's form: 12 hex digits of SHA-256 (`canonical_digest`, `combine`). It does not cover
/// the active scenarios' definitions (a scenario redefined through the API keeps its name).
pub(super) fn live_chaos(g: &Generation, snap: &Effective) -> LiveChaos {
    let live = snap.stack.live();
    let mut items: Vec<String> = live
        .entries()
        .into_iter()
        .map(|(k, l)| {
            let mut o = Map::new();
            if let Value::Object(v) = json!(super::chaos::ChaosView::from(l)) {
                o = v;
            }
            super::view::key_fields(g, &k, &mut o);
            Value::Object(o).to_string()
        })
        .collect();
    items.sort_unstable();
    let text = format!("[{}]", items.join(","));
    LiveChaos {
        scenarios: snap.stack.active().to_vec(),
        entries: live.entry_count(),
        digest: combine(&[("live".to_string(), canonical_digest(&text))]),
    }
}

/// **Private.** 12 hex digits over the compiled chaos a sweep runs under (RULES 140), whatever it
/// came from: file, inline and runtime scenarios and the live entries, after the scenario choice
/// and the stacking. Each compiled key (`EffectiveChaos::entries` and `replica_at_entries`) is
/// written by name (`key_fields`, so the config's node order does not matter) with its resolved
/// effect: latency, jitter and the outage start in integer nanoseconds, `down`, the bandwidth cap,
/// and the failure bands in compiled order (a higher layer's band comes first, so a stack order
/// that changes the effect changes the digest, and two stacks compiled to the same effect, which
/// give the same numbers, share it). The keys are a set: their texts are sorted, then hashed as the
/// fingerprint is (`canonical_digest`, `combine`).
pub(super) fn chaos_digest(g: &Generation, chaos: &EffectiveChaos) -> String {
    let ns = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
    let resolved = |r: &Resolved| {
        let faults: Vec<Value> = r
            .faults()
            .iter()
            .map(|f| json!({ "upto": f.upto, "kind": f.kind.as_str(), "status": f.status }))
            .collect();
        json!({
            "latency_ns": ns(r.latency_add),
            "jitter_ns": ns(r.jitter),
            "down": r.down,
            "down_from_ns": r.down_from.map(ns),
            "bandwidth_cap": r.bandwidth_cap,
            "faults": faults,
        })
    };
    let mut items: Vec<String> = chaos
        .entries()
        .into_iter()
        .map(|(k, r)| {
            let mut o = Map::new();
            super::view::key_fields(g, &k, &mut o);
            o.insert("chaos".into(), resolved(r));
            Value::Object(o).to_string()
        })
        .collect();
    items.extend(chaos.replica_at_entries().into_iter().map(|(n, rep, ep, r)| {
        json!({ "node": g.topo.nodes[n].name, "replica": rep, "endpoint": ep, "chaos": resolved(r) })
            .to_string()
    }));
    items.sort_unstable();
    let text = format!("[{}]", items.join(","));
    combine(&[("chaos".to_string(), canonical_digest(&text))])
}

/// The query of `GET /__kaanal/sim/runs/{id}/report`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportQuery {
    /// `markdown` (default) or `json`.
    format: Option<String>,
}

/// `GET /__kaanal/sim/runs/{id}/report` — a finished sweep's or plan's report (its `kind` decides): the Markdown
/// (`text/markdown; charset=utf-8`), or with `?format=json` its data alone.
///
/// # Errors
/// 400 for a bad `format`; 404 for an unknown id; 409 while the sweep or plan runs, for a failed one
/// (the body carries why) and for a plain run; 500 for a done record missing its recipe.
pub async fn get_run_report(
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
    // the record's kind names what it is that has no report yet
    let what = if rec.kind == "plan" { "plan" } else { "sweep" };
    let rep = report(&rec, env!("CARGO_PKG_VERSION")).map_err(|e| match e {
        ReportError::NotAReport => reject(
            StatusCode::CONFLICT,
            "run reports are made in Studio; only a sweep or a plan has a report here",
        ),
        ReportError::Running => {
            reject(StatusCode::CONFLICT, &format!("the {what} is not finished"))
        }
        ReportError::Failed(why) => reject(
            StatusCode::CONFLICT,
            &format!("the {what} failed and has no report: {why}"),
        ),
        ReportError::Internal(field) => reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("the {what} record has no {field}"),
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
