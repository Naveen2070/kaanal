//! Is an instance running: the lock probe, the `run.json` reader and the stale-state cleanup (spec M9, M16, M17).

use super::{
    Duration, Fail, Instant, IpAddr, Ipv4Addr, Path, STARTUP_WAIT, SocketAddr, Value, err,
};

/// **Private.** Whether some process holds the instance lock.
#[derive(Debug, PartialEq, Eq)]
pub enum Liveness {
    Running,
    Free,
}

/// **Private.** The `startup` object of `run.json` (M15): what `up` printed beyond the URLs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Startup {
    pub auth: Option<String>,
    pub seed: Option<u64>,
    pub warnings: Vec<String>,
    /// Started with no system (`up --blank`, `studio`): its UI link has no Admin tab to open on.
    pub blank: bool,
}

/// **Private.** The parsed `run.json`.
#[derive(Debug)]
pub struct Run {
    pub addr: SocketAddr,
    pub token: String,
    pub pid: Option<u32>,
    pub startup: Startup,
}

/// **Private.** Tries the instance lock without keeping it. `Free` means no process holds it (the
/// OS drops a dead process's lock, so this is true after a crash too); `WouldBlock` is `Running`.
/// Any other failure is unknown and an error, never "stale": acting on a guess could delete a
/// live instance's state.
pub fn lock_probe(state_dir: &Path) -> Result<Liveness, Fail> {
    Ok(match probe(state_dir)? {
        Probe::Running => Liveness::Running,
        Probe::Free(_) => Liveness::Free,
    })
}

/// **Private.** [`Liveness`] that keeps the shared lock while `Free`, so a new `up` cannot take the
/// exclusive lock between the probe and what the caller does about it (deleting stale state).
pub(super) enum Probe {
    Running,
    Free(Option<Guard>),
}

/// **Private.** A held shared lock, released explicitly on drop.
pub(super) struct Guard(std::fs::File);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub(super) fn probe(state_dir: &Path) -> Result<Probe, Fail> {
    let path = state_dir.join("lock");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    let f = match opts.open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !state_dir.join("run.json").exists() {
                return Ok(Probe::Free(None));
            }
            // A run.json with no lock file: create the lock so the caller has a guard to hold
            // while it decides, instead of deleting state unguarded.
            opts.create(true)
                .open(&path)
                .map_err(|e| err(format!("{}: {e}", path.display())))?
        }
        Err(e) => return Err(err(format!("{}: {e}", path.display()))),
    };
    // Shared, so two probes never make each other look "running"; the holder's lock is exclusive.
    // `up` retries its exclusive lock briefly (see `lock_instance`), so a probe never blocks it for long.
    match f.try_lock_shared() {
        Ok(()) => Ok(Probe::Free(Some(Guard(f)))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Probe::Running),
        Err(std::fs::TryLockError::Error(e)) => Err(err(format!("{}: {e}", path.display()))),
    }
}

/// **Private.** Reads `run.json`; any malformed piece gives `None`. A file without `host` (older
/// versions) means `127.0.0.1`; unknown fields are ignored.
pub fn read_run(state_dir: &Path) -> Option<Run> {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(state_dir.join("run.json")).ok()?).ok()?;
    let port = u16::try_from(v["port"].as_u64()?).ok()?;
    let host = match v.get("host") {
        Some(h) => h.as_str()?.parse().ok()?,
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    let s = &v["startup"];
    Some(Run {
        addr: SocketAddr::new(host, port),
        token: v["token"].as_str()?.to_string(),
        pid: v["pid"].as_u64().and_then(|p| u32::try_from(p).ok()),
        startup: Startup {
            auth: s["auth"].as_str().map(str::to_string),
            seed: s["seed"].as_u64(),
            warnings: s["warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| w.as_str().map(str::to_string))
                .collect(),
            blank: s["blank"].as_bool().unwrap_or(false),
        },
    })
}

/// **Private.** [`read_run`], retried every 100 ms for up to `budget`: the file appears a moment
/// after the lock is taken, when the server has bound (M15, M17).
pub fn read_run_wait(state_dir: &Path, budget: Duration) -> Option<Run> {
    let end = Instant::now() + budget;
    loop {
        if let Some(r) = read_run(state_dir) {
            return Some(r);
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// **Private.** Deletes `run.json` only if it still holds `token`: a newer instance may have
/// replaced the file since the caller read it.
pub fn remove_stale(state_dir: &Path, token: &str) {
    if read_run(state_dir).is_some_and(|r| r.token == token) {
        let _ = std::fs::remove_file(state_dir.join("run.json"));
    }
}

/// **Private.** The running instance, or the failure a command that needs one reports (M9, M16,
/// M17): stale or absent is exit 2; locked but no state file after the startup window is exit 1.
pub fn require_instance(state_dir: &Path) -> Result<Run, Fail> {
    require_within(state_dir, STARTUP_WAIT)
}

/// **Private.** [`require_instance`] with the startup window as a parameter (tests use a short one).
pub(super) fn require_within(state_dir: &Path, budget: Duration) -> Result<Run, Fail> {
    match probe(state_dir)? {
        // The guard is held to the end of this arm: no `up` can start between the probe and the delete.
        Probe::Free(_guard) => {
            let file = state_dir.join("run.json");
            match read_run(state_dir) {
                Some(r) => {
                    remove_stale(state_dir, &r.token);
                    Err(Fail(
                        2,
                        "kaanal is not running (removed stale state from a stopped instance)"
                            .into(),
                    ))
                }
                None if file.exists() => {
                    let _ = std::fs::remove_file(&file);
                    Err(Fail(
                        2,
                        "kaanal is not running (removed unreadable state from a stopped instance)"
                            .into(),
                    ))
                }
                None => Err(Fail(
                    2,
                    "kaanal is not running (no .kaanal/run.json)".into(),
                )),
            }
        }
        // Never delete anything while the lock is held: it may be an instance still starting.
        Probe::Running => read_run_wait(state_dir, budget)
            .ok_or_else(|| err("kaanal is starting or its state file is missing")),
    }
}

/// **Private.** The failure for a request that got no answer: with the lock held the instance
/// is alive but not answering (exit 1); with it free it is gone (exit 2) and its stale state goes.
pub fn transport_failure(state_dir: &Path, addr: SocketAddr, why: &str) -> Fail {
    match probe(state_dir) {
        Ok(Probe::Running) => {
            let pid = read_run(state_dir)
                .and_then(|r| r.pid)
                .map_or("unknown".to_string(), |p| p.to_string());
            err(format!(
                "kaanal is running (pid {pid}) but its control API did not answer: {why}"
            ))
        }
        Ok(Probe::Free(_guard)) => {
            if let Some(r) = read_run(state_dir) {
                remove_stale(state_dir, &r.token);
            }
            Fail(2, format!("kaanal is not running on {addr}: {why}"))
        }
        Err(e) => e,
    }
}

/// **Private.** What `down` finds when the lock is free: removes the stale state (under the shared
/// lock, so no `up` can start in between) and returns the line to print. `None` means the lock is
/// held, so an instance is alive or starting and nothing was touched.
pub fn clear_stale(state_dir: &Path) -> Result<Option<String>, Fail> {
    let Probe::Free(_guard) = probe(state_dir)? else {
        return Ok(None);
    };
    let file = state_dir.join("run.json");
    Ok(Some(match read_run(state_dir) {
        Some(r) => {
            remove_stale(state_dir, &r.token);
            r.pid.map_or_else(
                || "kaanal is not running (removed stale state)".to_string(),
                |p| format!("kaanal is not running (removed stale state from pid {p})"),
            )
        }
        None if file.exists() => {
            let _ = std::fs::remove_file(&file);
            "kaanal is not running (removed unreadable state)".to_string()
        }
        None => "kaanal is not running".to_string(),
    }))
}

/// **Private.** Polls the lock every 50 ms until it is free (true) or `budget` passes (false). An
/// unusable lock counts as not free: `down` must never report "stopped" on a guess.
pub fn wait_lock_free(state_dir: &Path, budget: Duration) -> bool {
    let end = Instant::now() + budget;
    loop {
        if matches!(lock_probe(state_dir), Ok(Liveness::Free)) {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
