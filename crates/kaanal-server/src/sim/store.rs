//! The runs Studio started: one at a time, the last eight kept, summaries and a log sample. A
//! load sweep is a record of kind `sweep` under the same slot, its points kept as they finish.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use kaanal_core::cost::CostReport;
use kaanal_sim::Summary;
use kaanal_sim::sweep::{Point, SweepResult};
use serde::Serialize;

use super::log::LogSample;

/// Runs kept, oldest dropped first.
const KEEP: usize = 8;

/// One run, as `GET /sim/runs/{id}` answers it.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    /// `s1`, `s2`, ...; never reused.
    pub id: String,
    /// `run` (`POST /sim/run`) or `sweep` (`POST /sim/sweep`).
    pub kind: &'static str,
    /// `running`, `done` or `failed`.
    pub state: &'static str,
    /// The generation the run used: an Apply later does not change what this report describes.
    pub generation: u64,
    /// The profile's name, or `inline`.
    pub profile: String,
    /// The scenario that supplied the chaos; `None` means the live chaos at the start.
    pub scenario: Option<String>,
    /// `applied` (the system in force) or `draft` (the draft text, simulated without Apply).
    pub source: &'static str,
    /// For a `draft` run, the draft's `rev` it was built from; `None` for `applied`.
    pub draft_rev: Option<u64>,
    /// Milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// Same, once the run ended.
    pub finished_ms: Option<u64>,
    /// The report, when `done`.
    pub summary: Option<Summary>,
    /// What the system costs an hour, estimated (RULES 126): the same sums as Studio's cost card,
    /// with an object store's requests priced at this run's visits. When `done`.
    pub cost: Option<CostReport>,
    /// Why it failed, when `failed`.
    pub error: Option<String>,
    /// A sweep's points in run order, growing while it runs; kept when it fails. Empty for a run.
    pub sweep_points: Vec<Point>,
    /// A sweep's finding, when `done`.
    pub sweep: Option<SweepResult>,
    /// A sweep's seed: the body's, else the config's. `None` for a run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// A sweep's highest load factor, as checked. `None` for a run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_factor: Option<f64>,
    /// A sweep's design fingerprint: the system it swept (see `crate::fingerprint`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The profile a sweep was posted with, when it was written out in the body (not a name from `traffic:`); kept so a report can
    /// say what was swept. Heavy, so `list()` strips it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_profile: Option<serde_json::Value>,
    /// The log sample, when `done`: served by `/log`, not part of the record's JSON. Shared,
    /// so reading a record does not copy it.
    #[serde(skip)]
    pub(crate) log: Arc<LogSample>,
}

/// What a sweep record keeps besides the run's own fields.
pub struct SweepMeta {
    /// The seed the draws used.
    pub seed: u64,
    /// The highest load factor on the ladder.
    pub max_factor: f64,
    /// The design fingerprint of the system in force.
    pub fingerprint: String,
    /// The posted profile when it was written out (label `inline`).
    pub inline_profile: Option<serde_json::Value>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    running: Option<String>,
    runs: VecDeque<RunRecord>,
}

/// The runs, behind one lock held only for bookkeeping, never across a run.
#[derive(Default)]
pub struct SimStore {
    inner: Mutex<Inner>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl SimStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a run record and takes the single run slot.
    ///
    /// # Errors
    /// The id of the run that already holds the slot.
    pub fn begin(
        &self,
        generation: u64,
        profile: &str,
        scenario: Option<&str>,
        source: &'static str,
        draft_rev: Option<u64>,
    ) -> Result<String, String> {
        self.begin_kind(generation, profile, scenario, source, draft_rev, None)
    }

    /// Starts a sweep record (always on the system in force) and takes the same single slot.
    ///
    /// # Errors
    /// The id of the run that already holds the slot.
    pub fn begin_sweep(
        &self,
        generation: u64,
        profile: &str,
        scenario: Option<&str>,
        meta: SweepMeta,
    ) -> Result<String, String> {
        self.begin_kind(generation, profile, scenario, "applied", None, Some(meta))
    }

    /// Creates the record under one lock: a sweep (`meta` given) carries its fields from the
    /// first poll on.
    fn begin_kind(
        &self,
        generation: u64,
        profile: &str,
        scenario: Option<&str>,
        source: &'static str,
        draft_rev: Option<u64>,
        meta: Option<SweepMeta>,
    ) -> Result<String, String> {
        let kind = if meta.is_some() { "sweep" } else { "run" };
        let mut i = self.lock();
        if let Some(id) = &i.running {
            return Err(id.clone());
        }
        i.next += 1;
        let id = format!("s{}", i.next);
        i.running = Some(id.clone());
        if i.runs.len() == KEEP {
            i.runs.pop_front();
        }
        i.runs.push_back(RunRecord {
            id: id.clone(),
            kind,
            state: "running",
            generation,
            profile: profile.to_string(),
            scenario: scenario.map(ToString::to_string),
            source,
            draft_rev,
            started_ms: now_ms(),
            finished_ms: None,
            summary: None,
            cost: None,
            error: None,
            sweep_points: Vec::new(),
            sweep: None,
            seed: meta.as_ref().map(|m| m.seed),
            max_factor: meta.as_ref().map(|m| m.max_factor),
            fingerprint: meta.as_ref().map(|m| m.fingerprint.clone()),
            inline_profile: meta.and_then(|m| m.inline_profile),
            log: Arc::default(),
        });
        Ok(id)
    }

    /// Ends run `id` with its result and frees the slot.
    pub fn finish(&self, id: &str, result: Result<(Summary, LogSample, CostReport), String>) {
        self.end(id, |r| match result {
            Ok((s, log, cost)) => {
                (r.state, r.summary) = ("done", Some(s));
                r.cost = Some(cost);
                r.log = Arc::new(log);
            }
            Err(e) => (r.state, r.error) = ("failed", Some(e)),
        });
    }

    /// Adds a finished point to sweep `id`.
    pub fn sweep_point(&self, id: &str, p: Point) {
        if let Some(r) = self.lock().runs.iter_mut().find(|r| r.id == id) {
            r.sweep_points.push(p);
        }
    }

    /// Ends sweep `id` with its finding and frees the slot.
    pub fn finish_sweep(&self, id: &str, result: Result<SweepResult, String>) {
        self.end(id, |r| match result {
            // the finding holds every point, sorted; the progress list is spent
            Ok(s) => {
                (r.state, r.sweep) = ("done", Some(s));
                r.sweep_points = Vec::new();
            }
            Err(e) => (r.state, r.error) = ("failed", Some(e)),
        });
    }

    /// Frees the slot if `id` holds it, and stamps and fills its record if it is still kept.
    fn end(&self, id: &str, fill: impl FnOnce(&mut RunRecord)) {
        let mut i = self.lock();
        if i.running.as_deref() == Some(id) {
            i.running = None;
        }
        if let Some(r) = i.runs.iter_mut().find(|r| r.id == id) {
            r.finished_ms = Some(now_ms());
            fill(r);
        }
    }

    /// One run, if it is still kept.
    pub fn get(&self, id: &str) -> Option<RunRecord> {
        self.lock().runs.iter().find(|r| r.id == id).cloned()
    }

    /// The kept runs, oldest first, without their summaries or sweep points.
    pub fn list(&self) -> Vec<RunRecord> {
        self.lock()
            .runs
            .iter()
            .map(|r| RunRecord {
                summary: None,
                cost: None,
                sweep_points: Vec::new(),
                sweep: None,
                inline_profile: None,
                log: Arc::default(),
                ..r.clone()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_slot_and_the_last_eight_are_kept() {
        let s = SimStore::default();
        let first = s.begin(1, "p", None, "applied", None).unwrap();
        assert_eq!(
            s.begin(1, "p", None, "applied", None),
            Err(first.clone()),
            "busy"
        );
        s.finish(&first, Err("boom".into()));
        assert_eq!(s.get(&first).unwrap().state, "failed");
        for _ in 0..8 {
            let id = s.begin(1, "p", None, "draft", Some(3)).unwrap();
            s.finish(&id, Err("x".into()));
        }
        assert!(
            s.get(&first).is_none(),
            "the ninth run pushed the first out"
        );
        assert_eq!(s.list().len(), 8);
    }

    fn meta() -> SweepMeta {
        SweepMeta {
            seed: 7,
            max_factor: 8.0,
            fingerprint: "0123456789ab".into(),
            inline_profile: Some(serde_json::json!({ "rps": 1 })),
        }
    }

    #[test]
    fn a_named_profile_sweep_has_no_inline_profile() {
        let s = SimStore::default();
        let id = s
            .begin_sweep(
                1,
                "peak",
                None,
                SweepMeta {
                    inline_profile: None,
                    ..meta()
                },
            )
            .unwrap();
        let r = s.get(&id).unwrap();
        assert_eq!(r.seed, Some(7));
        assert!(r.inline_profile.is_none());
    }

    fn point(k: f64) -> Point {
        Point {
            k,
            req_s: 100.0 * k,
            requests: 1000,
            goodput_req_s: 100.0 * k,
            error_rate: 0.0,
            p50_ms: 1.0,
            p95_ms: 2.0,
            p99_ms: 3.0,
            cause: None,
            nodes: Vec::new(),
            over: kaanal_sim::sweep::Over::default(),
        }
    }

    #[test]
    fn store_progress_grows_and_finish_keeps_eight() {
        let s = SimStore::default();
        let first = s.begin_sweep(1, "p", None, meta()).unwrap();
        assert_eq!(s.begin(1, "p", None, "applied", None), Err(first.clone()));
        assert_eq!(
            s.begin_sweep(1, "p", None, meta()),
            Err(first.clone()),
            "one slot"
        );
        let r = s.get(&first).unwrap();
        assert_eq!((r.kind, r.state, r.source), ("sweep", "running", "applied"));
        assert_eq!(r.sweep_points, []);
        assert!(r.sweep.is_none());
        assert_eq!((r.seed, r.max_factor), (Some(7), Some(8.0)));
        assert_eq!(r.fingerprint.as_deref(), Some("0123456789ab"));
        assert!(r.inline_profile.is_some());
        assert!(s.list()[0].inline_profile.is_none(), "the list strips it");
        s.sweep_point(&first, point(0.25));
        s.sweep_point(&first, point(0.5));
        let ks: Vec<f64> = s
            .get(&first)
            .unwrap()
            .sweep_points
            .iter()
            .map(|p| p.k)
            .collect();
        assert_eq!(ks, [0.25, 0.5], "the points grow in run order");
        s.finish_sweep(&first, Err("boom".into()));
        let r = s.get(&first).unwrap();
        assert_eq!(r.state, "failed");
        assert_eq!(r.sweep_points.len(), 2, "a failed sweep keeps what it ran");
        assert_eq!(s.list()[0].sweep_points, [], "the list strips the points");
        // a sweep that finishes OK hands its points to `sweep` and empties the progress list
        let second = s.begin_sweep(1, "p", None, meta()).unwrap();
        s.sweep_point(&second, point(0.25));
        s.finish_sweep(
            &second,
            Ok(SweepResult {
                label: "relative what-if",
                limit: kaanal_sim::sweep::Limit::default(),
                points: vec![point(0.25)],
                verdict: kaanal_sim::sweep::Verdict::HoldsTo {
                    k: 0.25,
                    req_s: 25.0,
                },
                recovers_at: None,
                stopped_at: None,
                first_to_saturate: None,
                dominant_cause: None,
                warnings: Vec::new(),
            }),
        );
        let r = s.get(&second).unwrap();
        assert_eq!(r.state, "done");
        assert_eq!(r.sweep_points, [], "the points moved into `sweep`");
        assert_eq!(r.sweep.unwrap().points.len(), 1);
        for _ in 0..8 {
            let id = s.begin(1, "p", None, "applied", None).unwrap();
            assert_eq!(s.get(&id).unwrap().kind, "run");
            s.finish(&id, Err("x".into()));
        }
        assert!(
            s.get(&first).is_none(),
            "the ninth record pushed the sweep out"
        );
        assert_eq!(s.list().len(), 8);
    }
}
