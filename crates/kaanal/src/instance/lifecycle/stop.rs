//! Stopping an instance: waiting for it, the forced stop, and the stop signals of the server process (M12).

use super::{
    Duration, Fail, Instant, Liveness, Path, PidCheck, Run, clear_stale, err, lock_probe, read_run,
    wait_lock_free,
};

/// **Private.** `$s.recv()` for a Windows console signal that registered, else never.
#[cfg(windows)]
macro_rules! recv_win {
    ($s:ident) => {
        async {
            match &mut $s {
                Ok(s) => {
                    s.recv().await;
                }
                Err(_) => std::future::pending().await,
            }
        }
    };
}

/// **Private.** [`run_or_gone`]'s answer.
pub enum Found {
    Run(Run),
    /// The instance is not (or no longer) running; the line to print.
    Gone(String),
}

/// **Private.** The running instance's `run.json`, waiting up to `budget` for it to appear. Each
/// round first checks for a vanished instance ([`clear_stale`]), so one that exits meanwhile is
/// `Gone`; a file that never appears under a held lock is exit 1.
pub fn run_or_gone(state_dir: &Path, budget: Duration) -> Result<Found, Fail> {
    let end = Instant::now() + budget;
    loop {
        if let Some(m) = clear_stale(state_dir)? {
            return Ok(Found::Gone(m));
        }
        if let Some(r) = read_run(state_dir) {
            return Ok(Found::Run(r));
        }
        if Instant::now() >= end {
            return Err(err("kaanal is starting or its state file is missing"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// **Private.** How `down` ended its force path.
#[derive(Debug, PartialEq, Eq)]
pub enum Forced {
    /// The instance went away by itself.
    Stopped,
    Killed,
}

/// **Private.** Is it still the same live instance? `Ok(true)` yes; `Ok(false)` it is gone (lock
/// free or file removed); `Err` the token or pid changed, so a different instance now owns the state.
pub(super) fn still_ours(state_dir: &Path, token: &str, pid: u32) -> Result<bool, Fail> {
    // A probe error is an error (exit 1), never "gone": we must not claim a stop on a guess.
    match lock_probe(state_dir)? {
        Liveness::Free => return Ok(false),
        Liveness::Running => {}
    }
    match read_run(state_dir) {
        // Lock held but run.json gone: the instance is mid-exit (it removes the file just before
        // releasing the lock), so `stopped` per spec M7.
        None => Ok(false),
        Some(r) if r.token == token && r.pid == Some(pid) => Ok(true),
        Some(_) => Err(err("kaanal instance changed while stopping; not killing")),
    }
}

/// **Private.** The force path of `down`: kill `pid` only if it is still the instance whose
/// `run.json` carried `token`, checked right before verifying and again right before killing, and
/// only if `verify` says it is kaanal. An instance that exits meanwhile is `Stopped`, not a failure.
/// `verify` and `kill` are parameters so tests can simulate the races.
pub fn force_stop(
    state_dir: &Path,
    token: &str,
    pid: u32,
    me: u32,
    verify: impl Fn(u32) -> PidCheck,
    kill: impl Fn(u32) -> Result<(), String>,
) -> Result<Forced, Fail> {
    if pid == me {
        return Err(err("refusing to stop myself (run.json names this process)"));
    }
    let gone = || matches!(lock_probe(state_dir), Ok(Liveness::Free));
    if !still_ours(state_dir, token, pid)? {
        return Ok(Forced::Stopped);
    }
    match verify(pid) {
        PidCheck::IsKaanal => {}
        _ if gone() => return Ok(Forced::Stopped),
        PidCheck::Other(name) => {
            return Err(err(format!(
                "kaanal did not stop, and pid {pid} is not a kaanal process ({name}); not killing it"
            )));
        }
        PidCheck::Unknown(why) => {
            return Err(err(format!(
                "kaanal did not stop, and pid {pid} could not be verified as kaanal; not killing it: {why}"
            )));
        }
    }
    if !still_ours(state_dir, token, pid)? {
        return Ok(Forced::Stopped);
    }
    if let Err(e) = kill(pid) {
        if gone() {
            return Ok(Forced::Stopped);
        }
        return Err(err(format!("could not kill pid {pid}: {e}")));
    }
    if !wait_lock_free(state_dir, Duration::from_secs(2)) {
        return Err(err(format!(
            "killed pid {pid} but the instance lock is still held"
        )));
    }
    // The kill worked; tidying the state is best effort and must not turn that into a failure.
    let _ = clear_stale(state_dir);
    Ok(Forced::Killed)
}

/// **Private.** Registers the stop handlers NOW and returns the future that waits for the first
/// stop request: unix SIGTERM or SIGINT, Windows Ctrl+C, Ctrl+Break, console close or system
/// shutdown (M12). Call it before anything is published (`run.json`, the URLs), inside the runtime,
/// so an early signal is queued instead of killing the process undrained. A signal that cannot be
/// registered never fires. The drain is bounded at 5 s by the caller; a second Ctrl+C during it is
/// swallowed on unix and terminates the process on Windows.
pub fn stop_signal() -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    let (mut term, mut int) = {
        use tokio::signal::unix::{SignalKind, signal};
        (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        )
    };
    #[cfg(windows)]
    let (mut c, mut b, mut cl, mut sh) = {
        use tokio::signal::windows as w;
        (
            w::ctrl_c(),
            w::ctrl_break(),
            w::ctrl_close(),
            w::ctrl_shutdown(),
        )
    };
    async move {
        #[cfg(unix)]
        tokio::select! {
            () = recv_unix(&mut term) => {},
            () = recv_unix(&mut int) => {},
        }
        #[cfg(windows)]
        tokio::select! {
            () = recv_win!(c) => {},
            () = recv_win!(b) => {},
            () = recv_win!(cl) => {},
            () = recv_win!(sh) => {},
        }
    }
}

#[cfg(unix)]
pub(super) async fn recv_unix(s: &mut std::io::Result<tokio::signal::unix::Signal>) {
    match s {
        Ok(s) => {
            s.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

/// **Private.** Unix, for `--detached`: swallow SIGHUP so closing the terminal does not kill the
/// daemon, which is what `nohup` does (M11). Needs a running tokio runtime. No-op on Windows.
pub fn ignore_hangup() {
    #[cfg(unix)]
    if let Ok(mut hup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
        tokio::spawn(async move { while hup.recv().await.is_some() {} });
    }
}
