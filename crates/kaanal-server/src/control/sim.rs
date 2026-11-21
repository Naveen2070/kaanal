//! The simulation routes: `POST /sim/run`, `GET /sim/runs`, `GET /sim/runs/{id}`.
//!
//! A run is started and answered with `202` and a `Location`; the client polls the record. One
//! run at a time (`409` with the running id otherwise), under `spawn_blocking`, with the server
//! caps ([`kaanal_sim::Limits::server`]), keeping a summary and dropping the hop records. A load
//! sweep (`POST /sim/sweep`, in `sweep.rs`) shares the slot, the records and the helpers here.

use super::*;
use crate::sim::draft_run::{DraftRunError, draft_job};
use crate::sim::log;
use crate::sim::profile::{Profile, RunRequest, SimError, to_sim};
use crate::sim::run::{Job, Slot, check, execute_run};

/// The accepted body of `POST /__kaanal/sim/run`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    /// A profile name from `traffic:`, or a profile written out.
    profile: Value,
    /// Simulated seconds (default: the profile's, else 60).
    #[serde(default)]
    duration_s: Option<f64>,
    /// Simulated seconds before results count (default: the profile's, else a tenth).
    #[serde(default)]
    warmup_s: Option<f64>,
    /// Seed of the draws (default the config's `seed:`).
    #[serde(default)]
    seed: Option<u64>,
    /// Run under this scenario's chaos alone, instead of the live chaos.
    #[serde(default)]
    scenario: Option<String>,
    /// `applied` (default) simulates the system in force; `draft` simulates the draft text,
    /// without Apply.
    #[serde(default)]
    source: Option<String>,
}

/// **Private.** Seconds from a body as a `Duration`; `None` when absent.
fn secs(name: &str, v: Option<f64>) -> Result<Option<Duration>, Reject> {
    v.map(|s| {
        Duration::try_from_secs_f64(s).map_err(|_| {
            reject(
                StatusCode::BAD_REQUEST,
                &format!("{name} must be a number of seconds, 0 or more"),
            )
        })
    })
    .transpose()
}

pub(super) fn from_sim(e: &SimError) -> Reject {
    reject(e.status, &e.message)
}

/// **Private.** The body's `profile`, a name or a profile written out, with the label a record
/// shows for it (the name, or `inline`).
pub(super) fn parse_profile(v: Value) -> Result<(String, Profile), Reject> {
    // not an untagged enum: it would throw away the precise message of a bad inline profile
    match v {
        Value::String(n) => Ok((n.clone(), Profile::Named(n))),
        v @ Value::Object(_) => {
            let p = serde_json::from_value(v)
                .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad profile: {e}")))?;
            Ok(("inline".to_string(), Profile::Inline(p)))
        }
        _ => Err(reject(
            StatusCode::BAD_REQUEST,
            "profile must be a profile name or an object",
        )),
    }
}

/// **Private.** What [`applied_inputs`] hands back.
pub(super) struct Applied {
    /// The system in force.
    pub(super) g: Arc<Generation>,
    /// The compiled chaos the run uses.
    pub(super) chaos: EffectiveChaos,
    /// The published state the live chaos was compiled from; `None` for a named scenario.
    pub(super) snap: Option<Arc<Effective>>,
    /// The scenarios in force (the named one, else the active stack) defined through the control
    /// API (`Source::Runtime`): no file or config holds them, so a command cannot repeat them.
    pub(super) runtime: Vec<String>,
}

/// **Private.** The system in force and the chaos `req` runs under: the named scenario's alone,
/// else the live chaos, with the published state it was compiled from (`None` for a scenario), so
/// a sweep can record what was in force.
pub(super) fn applied_inputs(st: &AppState, req: &RunRequest) -> Result<Applied, Reject> {
    // the chaos and the generation as one pair: an Apply swaps under this same lock
    let (c, g) = locked(st)?;
    let runtime_of = |names: &[String]| -> Vec<String> {
        names
            .iter()
            .filter(|n| matches!(c.scenarios.get(n), Some((_, kaanal_core::Source::Runtime))))
            .cloned()
            .collect()
    };
    let (chaos, snap, runtime) = if let Some(name) = &req.scenario {
        let mut stack = kaanal_core::ChaosStack::default();
        stack
            .set_active(&c.scenarios, std::slice::from_ref(name))
            .map_err(|_| reject(StatusCode::NOT_FOUND, &format!("unknown scenario '{name}'")))?;
        let runtime = runtime_of(std::slice::from_ref(name));
        (stack.compile(&c.scenarios), None, runtime)
    } else {
        let snap = st.snapshot();
        let runtime = runtime_of(snap.stack.active());
        (snap.chaos.clone(), Some(snap), runtime)
    };
    drop(c);
    Ok(Applied {
        g,
        chaos,
        snap,
        runtime,
    })
}

/// **Private.** `202` with the id and a `Location` to poll.
pub(super) fn accepted(id: &str) -> Response {
    let mut resp = (
        StatusCode::ACCEPTED,
        Json(json!({ "id": id, "state": "running" })),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("/__kaanal/sim/runs/{id}")) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    resp
}

/// **Private.** `409` naming the run that holds the slot.
pub(super) fn busy(running: &str) -> Reject {
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": "a run is already in progress", "id": running })),
    )
}

/// `POST /__kaanal/sim/run` — starts a run.
///
/// # Errors
/// 400 for a bad body, profile or request over the caps, 404 for an unknown profile or scenario,
/// 409 when a run is in progress or there is no system.
pub async fn post_run(State(st): State<Arc<AppState>>, body: Bytes) -> Result<Response, Reject> {
    let b: RunBody = serde_json::from_slice(&body)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let (label, profile) = parse_profile(b.profile)?;
    let req = RunRequest {
        profile,
        duration: secs("duration_s", b.duration_s)?,
        warmup: secs("warmup_s", b.warmup_s)?,
        seed: b.seed,
        scenario: b.scenario,
    };
    let (g, chaos, source, draft_rev) = match b.source.as_deref() {
        None | Some("applied") => {
            let a = applied_inputs(&st, &req)?;
            (a.g, a.chaos, "applied", None)
        }
        Some("draft") => {
            let (job, rev) = draft_job(&st, &req).await.map_err(|e| match e {
                DraftRunError::Invalid(d) => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": d.message, "diagnostic": d })),
                ),
                DraftRunError::UnknownScenario(n) => {
                    reject(StatusCode::NOT_FOUND, &format!("unknown scenario '{n}'"))
                }
                DraftRunError::Busy => reject(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many drafts being built at once; retry",
                ),
                DraftRunError::Internal(m) => reject(StatusCode::INTERNAL_SERVER_ERROR, &m),
            })?;
            (job.g, job.chaos, "draft", Some(rev))
        }
        Some(_) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "source must be applied or draft",
            ));
        }
    };
    let mut planned = to_sim(&g, &req).map_err(|e| from_sim(&e))?;
    // the cheap checks first, with placeholder response sizes: sampling a body per journey is
    // real work and waits until the request is known to be accepted
    let mut job = Job {
        g,
        chaos,
        cfg: planned.cfg.clone(),
    };
    check(&job).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let id = st
        .sims
        .begin(job.g.id, &label, req.scenario.as_deref(), source, draft_rev)
        .map_err(|running| busy(&running))?;
    let slot = Slot::new(Arc::clone(&st), id.clone());
    tokio::task::spawn_blocking(move || {
        planned.fill_sizes(&job.g);
        job.cfg = planned.cfg;
        let result = execute_run(&job);
        slot.finish(result);
    });
    Ok(accepted(&id))
}

/// `GET /__kaanal/sim/runs` — the kept runs, without their summaries.
pub async fn list_runs(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "runs": st.sims.list() }))
}

/// `GET /__kaanal/sim/runs/{id}` — one run with its summary once done.
///
/// # Errors
/// 404 for an id that was never issued or has been pushed out of the last eight.
pub async fn get_run(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::sim::RunRecord>, Reject> {
    st.sims.get(&id).map(Json).ok_or_else(|| {
        reject(
            StatusCode::NOT_FOUND,
            "no such run (only the last eight are kept)",
        )
    })
}

/// The accepted body of `POST /__kaanal/sample`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleBody {
    /// The endpoint key, `"METHOD /path"`.
    endpoint: String,
    /// Seed of the data (default the config's `seed:`).
    #[serde(default)]
    seed: Option<u64>,
}

/// Largest sample returned, serialized.
const SAMPLE_CAP: usize = 256 * 1024;

/// `POST /__kaanal/sample` — one generated response body of an endpoint, from the generator and
/// seeds the mock uses. A route with no response schema answers `body: null`.
///
/// # Errors
/// 400 for a bad body, a bad key or a sample over 256 KiB; 404 for an endpoint that is not a
/// route; 409 without a system; 429 while two samples or skeletons are already being made.
pub async fn post_sample(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let b: SampleBody = serde_json::from_slice(&body)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    let key = endpoint_key(&b.endpoint).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    // generating a body is CPU work: off the async threads, a few at a time. The permit moves
    // into the job, so it is held for as long as the work lasts, even if the client leaves.
    let permit = Arc::clone(&st.work).try_acquire_owned().map_err(|_| {
        reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many samples at once; retry",
        )
    })?;
    let (g2, key2, seed) = (Arc::clone(&g), key.clone(), b.seed);
    let s = tokio::task::spawn_blocking(move || {
        let _held = permit;
        crate::sim::sample::sample(&g2, &key2, seed)
    })
    .await
    .map_err(|_| reject(StatusCode::INTERNAL_SERVER_ERROR, "the sample failed"))?
    .ok_or_else(|| reject(StatusCode::NOT_FOUND, &format!("no route '{key}'")))?;
    if s.bytes > SAMPLE_CAP {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            &format!("the sample is {} bytes; the limit is {SAMPLE_CAP}", s.bytes),
        ));
    }
    Ok(Json(json!({
        "endpoint": key,
        "status": s.status,
        "body": s.body,
        "bytes": s.bytes,
        "label": "relative what-if, not a prediction",
    })))
}

/// The query of `GET /__kaanal/sim/runs/{id}/log`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
    node: Option<String>,
    /// With `to`: only the hops on the wire from this node to that one.
    from: Option<String>,
    to: Option<String>,
    replica: Option<u16>,
    endpoint: Option<String>,
    level: Option<String>,
    request: Option<u64>,
    limit: Option<usize>,
    after: Option<usize>,
}

/// Lines a page may hold, and its default.
const LOG_MAX: usize = 1000;
const LOG_DEFAULT: usize = 200;

/// `GET /__kaanal/sim/runs/{id}/log` — a page of the run's log sample (see `sim::log`).
/// `limit` defaults to 200 and is clamped to 1000; `after` is the index of the last line
/// seen; `next` is the `after` of the next page, `null` at the end; `total` counts the stored
/// lines, before the filters.
///
/// # Errors
/// 400 for a bad `level` or `endpoint`; 404 for an unknown id; 409 while the run is running, for
/// a failed run (it has no log; the body carries its `error`), and for a sweep.
pub async fn get_run_log(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    q: Result<Query<LogQuery>, QueryRejection>,
) -> Result<Json<Value>, Reject> {
    let Query(q) = q.map_err(|e| reject(StatusCode::BAD_REQUEST, &e.body_text()))?;
    let level = match q.level.as_deref() {
        None => None,
        Some(l @ ("info" | "warn" | "error")) => Some(l.to_string()),
        Some(_) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "level must be info, warn or error",
            ));
        }
    };
    let wire = match (q.from, q.to) {
        (None, None) => None,
        (Some(a), Some(b)) => Some((a, b)),
        _ => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "from and to go together: they name one wire",
            ));
        }
    };
    let endpoint = q
        .endpoint
        .as_deref()
        .map(|e| endpoint_key(e).map_err(|m| reject(StatusCode::BAD_REQUEST, &m)))
        .transpose()?;
    let run = st.sims.get(&id).ok_or_else(|| {
        reject(
            StatusCode::NOT_FOUND,
            "no such run (only the last eight are kept)",
        )
    })?;
    if run.kind == "sweep" {
        return Err(reject(
            StatusCode::CONFLICT,
            "a sweep has no log; its points are in the record",
        ));
    }
    match run.state {
        "done" => {}
        "running" => return Err(reject(StatusCode::CONFLICT, "the run is still running")),
        _ => {
            let why = run.error.as_deref().unwrap_or("unknown");
            return Err(reject(
                StatusCode::CONFLICT,
                &format!("the run failed and has no log: {why}"),
            ));
        }
    }
    let f = log::Filter {
        node: q.node,
        wire,
        replica: q.replica,
        endpoint,
        level,
        request: q.request,
    };
    let limit = q.limit.unwrap_or(LOG_DEFAULT).clamp(1, LOG_MAX);
    let (lines, next) = log::page(&run.log.lines, &f, q.after, limit);
    let note = format!(
        "A sample in virtual time, not every request: all failed requests (up to {}), the {} \
         slowest, and an even spread of the rest, {} requests at most. Only requests that \
         arrived after the warmup are logged. Failure lines carry no replica (the engine does \
         not record it), so a replica filter skips them.{}",
        log::MAX_FAILED,
        log::SLOWEST,
        log::MAX_REQUESTS,
        if run.log.truncated {
            format!(
                " The log is capped at {} lines, so fewer requests were kept than the rule allows.",
                log::MAX_LINES
            )
        } else {
            String::new()
        }
    );
    Ok(Json(
        json!({ "lines": lines, "next": next, "total": run.log.lines.len(), "sampled": true, "note": note }),
    ))
}
