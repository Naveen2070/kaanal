//! Waiting for a started instance to answer: polling, the unready settle, and the existing-instance check.

use super::{
    Child, Duration, Fail, Instant, Liveness, Path, Run, SocketAddr, clean, err, lock_probe,
    read_run,
};

/// **Private.** Why [`wait_ready`] gave up.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadyError {
    ChildExited,
    TimedOut,
}

/// **Private.** One look at the starting child.
pub enum Step {
    Ready(Run),
    Exited,
    Pending,
}

/// **Private.** Calls `step` every `poll` until it says ready or exited, or `budget` passes.
pub fn poll_until(
    budget: Duration,
    poll: Duration,
    mut step: impl FnMut() -> Step,
) -> Result<Run, ReadyError> {
    let end = Instant::now() + budget;
    loop {
        match step() {
            Step::Ready(r) => return Ok(r),
            Step::Exited => return Err(ReadyError::ChildExited),
            Step::Pending => {}
        }
        if Instant::now() >= end {
            return Err(ReadyError::TimedOut);
        }
        std::thread::sleep(poll);
    }
}

/// **Private.** Whether `GET /__kaanal/health` on `addr` answers 200.
pub fn healthy(addr: SocketAddr) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .into();
    agent
        .get(&format!("http://{addr}/__kaanal/health"))
        .header("Host", &format!("localhost:{}", addr.port()))
        .call()
        .is_ok_and(|r| r.status().as_u16() == 200)
}

/// **Private.** Waits for the spawned `child` to be a live instance: its own `run.json` (the pid
/// matches), the instance lock held, and the health endpoint answering. Ends early when the child
/// has exited.
pub fn wait_ready(
    state_dir: &Path,
    child: &mut Child,
    budget: Duration,
    poll: Duration,
) -> Result<Run, ReadyError> {
    poll_until(budget, poll, || {
        if child.try_wait().ok().flatten().is_some() {
            return Step::Exited;
        }
        match read_run(state_dir) {
            Some(r)
                if r.pid == Some(child.id())
                    && matches!(lock_probe(state_dir), Ok(Liveness::Running))
                    && healthy(r.addr) =>
            {
                Step::Ready(r)
            }
            _ => Step::Pending,
        }
    })
}

/// **Private.** How an instance that did not answer in time ended up.
#[derive(Debug)]
pub enum Settled {
    /// It answered after all: leave it running (M3).
    Healthy(Run),
    /// It is stopped, by its own shutdown or by us.
    Stopped,
}

/// **Private.** Decides what to do with a child that missed the deadline. Healthy now: keep it.
/// Else ask it to shut down when `run.json` is its own, and wait for that; whatever is left
/// (ignored, foreign or no `run.json`) is killed: we hold its `Child`, so its identity is certain.
/// The steps are parameters so tests can inject each outcome.
pub fn settle_unready(
    healthy_run: impl FnOnce() -> Option<Run>,
    own_run: impl FnOnce() -> Option<Run>,
    shutdown: impl FnOnce(&Run),
    wait_stopped: impl FnOnce(),
    kill: impl FnOnce(),
) -> Settled {
    if let Some(r) = healthy_run() {
        return Settled::Healthy(r);
    }
    if let Some(r) = own_run() {
        shutdown(&r);
        wait_stopped();
    }
    // Always: a child that is slow to exit must not block the caller's `wait()`; killing one
    // that has already gone is a harmless error.
    kill();
    Settled::Stopped
}

/// **Private.** `run.json` if it is `pid`'s own (never another instance's).
pub fn run_of(state_dir: &Path, pid: u32) -> Option<Run> {
    read_run(state_dir).filter(|r| r.pid == Some(pid))
}

/// **Private.** A wait taken from a server-provided `timeout_ms`, clamped to the server's own 300 s
/// cap plus slack, so a garbled value can never overflow an `Instant` or wait for ever.
pub fn server_wait(ms: Option<u64>, default_ms: u64) -> Duration {
    Duration::from_millis(ms.unwrap_or(default_ms).min(310_000))
}

/// **Private.** After the child we spawned exited: a healthy instance that won a start race
/// (`Some`), none (`None`: it really failed), or an error when one holds the lock but does not
/// answer within `budget` (the remaining `--wait`, never a fresh one).
pub fn after_child_exit(
    budget: Duration,
    poll: Duration,
    look: impl FnMut() -> Result<Look, Fail>,
) -> Result<Option<Run>, Fail> {
    Ok(match existing_instance(budget, poll, look)? {
        Existing::Running(r) => Some(r),
        Existing::Free => None,
    })
}

/// **Private.** One look at the instance directory by `up -d`.
pub enum Look {
    /// No instance (stale state is cleaned).
    Free,
    /// An instance whose health endpoint answers.
    Healthy(Run),
    /// The lock is held but nothing answers: starting, draining, or wedged.
    Busy,
}

/// **Private.** What [`existing_instance`] found.
pub enum Existing {
    Running(Run),
    Free,
}

/// **Private.** Looks every `poll` for up to `budget`: a healthy instance is `Running`, no
/// instance is `Free`, and a busy one (starting or draining) is waited on, since it is neither
/// "running" nor safe to start beside. Still busy at the deadline is an error.
pub fn existing_instance(
    budget: Duration,
    poll: Duration,
    mut look: impl FnMut() -> Result<Look, Fail>,
) -> Result<Existing, Fail> {
    let end = Instant::now() + budget;
    loop {
        match look()? {
            Look::Free => return Ok(Existing::Free),
            Look::Healthy(r) => return Ok(Existing::Running(r)),
            Look::Busy => {}
        }
        if Instant::now() >= end {
            return Err(err(format!(
                "kaanal is running but not answering (still starting, or shutting down); gave up after {}",
                humantime::format_duration(budget)
            )));
        }
        std::thread::sleep(poll);
    }
}

/// **Private.** The last `lines` lines of the log, each cleaned (the log holds server text).
pub fn tail_log(path: &Path, lines: usize) -> String {
    // Reads the whole file; the log is fresh per `up -d`, so it is small on a failed start.
    let text = std::fs::read(path).map_or_default(|b| String::from_utf8_lossy(&b).into_owned());
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| clean(l))
        .collect::<Vec<_>>()
        .join("\n")
}
