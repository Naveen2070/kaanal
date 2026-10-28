//! The simulation routes: `POST /sim/run`, `GET /sim/runs`, `GET /sim/runs/{id}`.
//!
//! A run is started and answered with `202` and a `Location`; the client polls the record. One
//! run at a time (`409` with the running id otherwise), under `spawn_blocking`, with the server
//! caps ([`kaanal_sim::Limits::server`]), keeping a summary and dropping the hop records.

use super::*;
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

fn from_sim(e: &SimError) -> Reject {
    reject(e.status, &e.message)
}

/// `POST /__kaanal/sim/run` — starts a run.
///
/// # Errors
/// 400 for a bad body, profile or request over the caps, 404 for an unknown profile or scenario,
/// 409 when a run is in progress or there is no system.
pub async fn post_run(State(st): State<Arc<AppState>>, body: Bytes) -> Result<Response, Reject> {
    let b: RunBody = serde_json::from_slice(&body)
        .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
    // not an untagged enum: it would throw away the precise message of a bad inline profile
    let (label, profile) = match b.profile {
        Value::String(n) => (n.clone(), Profile::Named(n)),
        v @ Value::Object(_) => {
            let p = serde_json::from_value(v)
                .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad profile: {e}")))?;
            ("inline".to_string(), Profile::Inline(p))
        }
        _ => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                "profile must be a profile name or an object",
            ));
        }
    };
    let req = RunRequest {
        profile,
        duration: secs("duration_s", b.duration_s)?,
        warmup: secs("warmup_s", b.warmup_s)?,
        seed: b.seed,
        scenario: b.scenario,
    };
    // the chaos and the generation as one pair: an Apply swaps under this same lock
    let (c, g) = locked(&st)?;
    let chaos = if let Some(name) = &req.scenario {
        let mut stack = kaanal_core::ChaosStack::default();
        stack
            .set_active(&c.scenarios, std::slice::from_ref(name))
            .map_err(|_| reject(StatusCode::NOT_FOUND, &format!("unknown scenario '{name}'")))?;
        stack.compile(&c.scenarios)
    } else {
        st.snapshot().chaos.clone()
    };
    drop(c);
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
        .begin(job.g.id, &label, req.scenario.as_deref())
        .map_err(|running| {
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": "a run is already in progress", "id": running })),
            )
        })?;
    let slot = Slot::new(Arc::clone(&st), id.clone());
    tokio::task::spawn_blocking(move || {
        planned.fill_sizes(&job.g);
        job.cfg = planned.cfg;
        let result = execute_run(&job);
        slot.finish(result);
    });
    let mut resp = (
        StatusCode::ACCEPTED,
        Json(json!({ "id": id, "state": "running" })),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("/__kaanal/sim/runs/{id}")) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    Ok(resp)
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
