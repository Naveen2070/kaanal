//! The runs Studio started: one at a time, the last eight kept, summaries only.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use kaanal_sim::Summary;
use serde::Serialize;

/// Runs kept, oldest dropped first.
const KEEP: usize = 8;

/// One run, as `GET /sim/runs/{id}` answers it.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    /// `s1`, `s2`, ...; never reused.
    pub id: String,
    /// `running`, `done` or `failed`.
    pub state: &'static str,
    /// The generation the run used: an Apply later does not change what this report describes.
    pub generation: u64,
    /// The profile's name, or `inline`.
    pub profile: String,
    /// The scenario that supplied the chaos; `None` means the live chaos at the start.
    pub scenario: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// Same, once the run ended.
    pub finished_ms: Option<u64>,
    /// The report, when `done`.
    pub summary: Option<Summary>,
    /// Why it failed, when `failed`.
    pub error: Option<String>,
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

    /// Starts a record and takes the single run slot.
    ///
    /// # Errors
    /// The id of the run that already holds the slot.
    pub fn begin(
        &self,
        generation: u64,
        profile: &str,
        scenario: Option<&str>,
    ) -> Result<String, String> {
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
            state: "running",
            generation,
            profile: profile.to_string(),
            scenario: scenario.map(ToString::to_string),
            started_ms: now_ms(),
            finished_ms: None,
            summary: None,
            error: None,
        });
        Ok(id)
    }

    /// Ends run `id` with its result and frees the slot.
    pub fn finish(&self, id: &str, result: Result<Summary, String>) {
        let mut i = self.lock();
        if i.running.as_deref() == Some(id) {
            i.running = None;
        }
        if let Some(r) = i.runs.iter_mut().find(|r| r.id == id) {
            r.finished_ms = Some(now_ms());
            match result {
                Ok(s) => (r.state, r.summary) = ("done", Some(s)),
                Err(e) => (r.state, r.error) = ("failed", Some(e)),
            }
        }
    }

    /// One run, if it is still kept.
    pub fn get(&self, id: &str) -> Option<RunRecord> {
        self.lock().runs.iter().find(|r| r.id == id).cloned()
    }

    /// The kept runs, oldest first, without their summaries.
    pub fn list(&self) -> Vec<RunRecord> {
        self.lock()
            .runs
            .iter()
            .map(|r| RunRecord {
                summary: None,
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
        let first = s.begin(1, "p", None).unwrap();
        assert_eq!(s.begin(1, "p", None), Err(first.clone()), "busy");
        s.finish(&first, Err("boom".into()));
        assert_eq!(s.get(&first).unwrap().state, "failed");
        for _ in 0..8 {
            let id = s.begin(1, "p", None).unwrap();
            s.finish(&id, Err("x".into()));
        }
        assert!(
            s.get(&first).is_none(),
            "the ninth run pushed the first out"
        );
        assert_eq!(s.list().len(), 8);
    }
}
