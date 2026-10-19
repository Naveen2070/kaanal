//! Is an instance running, and how to stop one: the lock-based liveness check (spec M9, M16, M17),
//! the `run.json` reader, and the stop signals (M12). Every function takes the state directory, so
//! unit tests use a temp dir.

use std::ffi::OsString;
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{Fail, clean, err};

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

/// **Private.** How long a command waits for `run.json` while the lock is held (M17).
pub const STARTUP_WAIT: Duration = Duration::from_secs(5);

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
enum Probe {
    Running,
    Free(Option<Guard>),
}

/// **Private.** A held shared lock, released explicitly on drop.
struct Guard(std::fs::File);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn probe(state_dir: &Path) -> Result<Probe, Fail> {
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
fn require_within(state_dir: &Path, budget: Duration) -> Result<Run, Fail> {
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

/// **Private.** What a pid is, as far as `down` may act on it.
#[derive(Debug, PartialEq, Eq)]
pub enum PidCheck {
    IsKaanal,
    /// Another program's image name.
    Other(String),
    /// The OS tool failed or showed nothing; never a reason to kill.
    Unknown(String),
}

/// **Private.** The image name from `tasklist /FO CSV /NH` output (`"kaanal.exe","123",...`); the
/// "INFO: No tasks are running..." line, or anything else unquoted, gives `None`.
#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "only the Windows verify_pid reads tasklist")
)]
pub fn parse_tasklist_csv(out: &str) -> Option<String> {
    let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let name = line.strip_prefix('"')?.split('"').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// **Private.** The command name from `ps -p N -o comm=`: trimmed, last path component.
#[cfg_attr(
    all(windows, not(test)),
    expect(dead_code, reason = "only the unix verify_pid reads ps")
)]
pub fn parse_ps_comm(out: &str) -> Option<String> {
    let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let base = line.rsplit('/').next()?.trim();
    (!base.is_empty()).then(|| base.to_string())
}

/// **Private.** An OS tool by full path (System32 on Windows, /bin or /usr/bin on unix), so a
/// planted earlier-PATH executable is never run.
fn os_tool(name: &str) -> Result<std::path::PathBuf, String> {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").map_err(|_| "SystemRoot is not set")?;
        Ok(std::path::PathBuf::from(format!(
            r"{root}\System32\{name}.exe"
        )))
    }
    #[cfg(not(windows))]
    {
        ["/bin", "/usr/bin"]
            .iter()
            .map(|d| Path::new(d).join(name))
            .find(|p| p.exists())
            .ok_or_else(|| format!("{name} not found in /bin or /usr/bin"))
    }
}

/// **Private.** A command for an OS tool. On Windows it gets no console window: the `up -d` daemon
/// has none, and a console program started from it would make Windows build one (measured ~15 s).
#[cfg(windows)]
pub fn os_command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new(path);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// **Private.** A command for an OS tool (no console window to avoid on this platform).
#[cfg(not(windows))]
pub fn os_command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    Command::new(path)
}

/// **Private.** Runs an OS tool: its stdout on success, its trimmed output as the error otherwise.
fn run_tool(name: &str, args: &[&str]) -> Result<String, String> {
    let out = os_command(os_tool(name)?)
        .args(args)
        .output()
        .map_err(|e| format!("{name}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{name} failed: {} {}", text.trim(), err.trim()))
    }
}

/// **Private.** The file stem of this very executable (a renamed or hashed copy of kaanal).
fn own_stem() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// **Private.** Whether a process image name is kaanal: the standard name (`want`, with or without
/// `.exe`) or this executable's own stem, case-insensitively. A deliberate widening of spec M10:
/// a renamed binary can still be stopped by itself, and still never anyone else's.
///
/// `comm_limit` is the platform's cap on the name `ps` shows (Linux `comm` is 15 chars): a longer
/// own stem is compared truncated to it, so `kaanal-<16 hex>` matches `kaanal-xxxxxxxx`.
fn is_kaanal_image(name: &str, want: &str, own: &str, comm_limit: Option<usize>) -> bool {
    let strip = |s: &str| -> String {
        match s.len().checked_sub(4).and_then(|i| s.get(i..)) {
            Some(ext) if ext.eq_ignore_ascii_case(".exe") => s[..s.len() - 4].to_string(),
            _ => s.to_string(),
        }
    };
    let (stem, want) = (strip(name), strip(want));
    let own: String = match comm_limit {
        Some(n) => own.chars().take(n).collect(),
        None => own.to_string(),
    };
    stem.eq_ignore_ascii_case(&want) || (!own.is_empty() && stem.eq_ignore_ascii_case(&own))
}

/// **Private.** Linux truncates `ps -o comm=` to 15 characters; elsewhere names are whole.
const COMM_LIMIT: Option<usize> = if cfg!(target_os = "linux") {
    Some(15)
} else {
    None
};

/// **Private.** Whether `pid` is a kaanal process, by its image name. Only `IsKaanal` allows a kill.
pub fn verify_pid(pid: u32) -> PidCheck {
    let p = pid.to_string();
    #[cfg(windows)]
    let (res, parse, want): (_, fn(&str) -> Option<String>, _) = (
        run_tool(
            "tasklist",
            &["/FI", &format!("PID eq {p}"), "/FO", "CSV", "/NH"],
        ),
        parse_tasklist_csv,
        "kaanal.exe",
    );
    #[cfg(not(windows))]
    let (res, parse, want): (_, fn(&str) -> Option<String>, _) = (
        run_tool("ps", &["-p", &p, "-o", "comm="]),
        parse_ps_comm,
        "kaanal",
    );
    match res {
        Err(e) => PidCheck::Unknown(e),
        Ok(out) => match parse(&out) {
            Some(n) if is_kaanal_image(&n, want, &own_stem(), COMM_LIMIT) => PidCheck::IsKaanal,
            Some(n) => PidCheck::Other(n),
            None => PidCheck::Unknown(format!("no process with pid {pid}")),
        },
    }
}

/// **Private.** Force-kills `pid` (`taskkill /F` / `kill -KILL`). Call only after [`verify_pid`].
pub fn kill_pid(pid: u32) -> Result<(), String> {
    let p = pid.to_string();
    #[cfg(windows)]
    let res = run_tool("taskkill", &["/PID", &p, "/F"]);
    #[cfg(not(windows))]
    let res = run_tool("kill", &["-KILL", &p]);
    res.map(drop)
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
fn still_ours(state_dir: &Path, token: &str, pid: u32) -> Result<bool, Fail> {
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
async fn recv_unix(s: &mut std::io::Result<tokio::signal::unix::Signal>) {
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

/// **Private.** Moves `kaanal.log` to `kaanal.log.1` (replacing the old backup) and creates a fresh
/// `kaanal.log` (0600 on unix; the state dir is already private). Returns the log's path.
pub fn rotate_log(state_dir: &Path) -> Result<PathBuf, Fail> {
    let log = state_dir.join("kaanal.log");
    let mut o = std::fs::OpenOptions::new();
    let mut truncate = false;
    match std::fs::rename(&log, state_dir.join("kaanal.log.1")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            // Something holds the old log open without allowing a rename: keep going, but
            // lose the old text rather than refuse to start.
            eprintln!(
                "warning: could not keep {} as kaanal.log.1 ({e}); truncating it",
                log.display()
            );
            truncate = true;
        }
    }
    if truncate {
        o.write(true).truncate(true);
    } else {
        o.append(true);
    }
    o.create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(&log)
        .map_err(|e| err(format!("{}: {e}", log.display())))?;
    Ok(log)
}

/// **Private.** Starts `exe args` as a background process in `cwd`: no stdin, stdout and stderr the
/// log, and detached from the caller's console and process group so closing the terminal or
/// pressing Ctrl+C there does not reach it. It must hold none of the caller's pipes (pipe test).
pub fn spawn_detached(exe: &Path, args: &[OsString], log: File, cwd: &Path) -> Result<Child, Fail> {
    let again = log.try_clone().map_err(|e| err(format!("log file: {e}")))?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(again);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        const BASE: u32 = 0x0000_0008 | 0x0000_0200 | 0x0800_0000;
        const BREAKAWAY: u32 = 0x0100_0000; // CREATE_BREAKAWAY_FROM_JOB
        // Outlive a job object that kills its members at close (a terminal, an IDE); a job that
        // forbids breakaway refuses with access denied, so retry inside the job.
        // std spawns with bInheritHandles=TRUE: without this the daemon would also inherit the
        // caller's stdout/stderr pipes and `$(kaanal up -d)` would never see EOF (R1.46).
        kaanal_winhandle::without_inheritable_stdio(|| {
            match cmd.creation_flags(BASE | BREAKAWAY).spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    cmd.creation_flags(BASE).spawn()
                }
                r => r,
            }
        })
    };
    #[cfg(not(windows))]
    let spawned = cmd.spawn();
    spawned.map_err(|e| err(format!("could not start {}: {e}", exe.display())))
}

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
    // ponytail: reads the whole file; the log is fresh per `up -d`, so it is small on a failed start.
    let text = std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| clean(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn rotate_log_truncates_when_the_old_log_cannot_be_renamed() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = dir("rot-locked");
        // read+write sharing but no FILE_SHARE_DELETE: renaming the file fails
        let mut held = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(3)
            .open(d.join("kaanal.log"))
            .unwrap();
        std::io::Write::write_all(&mut held, b"old").unwrap();
        let log = rotate_log(&d).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
        assert!(!d.join("kaanal.log.1").exists());
    }

    fn fake_run(pid: u32) -> Run {
        Run {
            addr: "127.0.0.1:1".parse().unwrap(),
            token: "t".into(),
            pid: Some(pid),
            startup: Startup::default(),
        }
    }

    #[test]
    fn settle_unready_keeps_a_healthy_child_and_otherwise_always_ends_it() {
        use std::cell::Cell;
        let (shut, killed) = (Cell::new(false), Cell::new(false));
        let waited = Cell::new(false);
        let go = |healthy: bool, own: bool| {
            shut.set(false);
            killed.set(false);
            waited.set(false);
            settle_unready(
                || healthy.then(|| fake_run(1)),
                || own.then(|| fake_run(1)),
                |_| shut.set(true),
                || waited.set(true),
                || killed.set(true),
            )
        };
        assert!(matches!(go(true, true), Settled::Healthy(_)));
        assert!(!shut.get() && !killed.get());
        // own run.json: asked to shut down, waited for, and killed anyway (a slow exit must not hang us)
        assert!(matches!(go(false, true), Settled::Stopped));
        assert!(shut.get() && waited.get() && killed.get());
        // no (or foreign) run.json: no shutdown, killed
        go(false, false);
        assert!(!shut.get() && !waited.get() && killed.get());
    }

    #[test]
    fn existing_instance_waits_on_a_busy_one_and_decides_on_free_or_healthy() {
        let mut seq = vec![Look::Busy, Look::Busy, Look::Healthy(fake_run(9))];
        let r = existing_instance(Duration::from_secs(5), Duration::from_millis(1), || {
            Ok(seq.remove(0))
        });
        assert!(matches!(r, Ok(Existing::Running(r)) if r.pid == Some(9)));
        let mut seq = vec![Look::Busy, Look::Free];
        let r = existing_instance(Duration::from_secs(5), Duration::from_millis(1), || {
            Ok(seq.remove(0))
        });
        assert!(matches!(r, Ok(Existing::Free)));
        let r = existing_instance(Duration::from_millis(30), Duration::from_millis(5), || {
            Ok(Look::Busy)
        });
        let Err(Fail(1, m)) = r else {
            panic!("a busy instance at the deadline is an error");
        };
        assert!(m.contains("not answering"), "{m}");
    }

    #[test]
    fn server_waits_are_clamped() {
        assert_eq!(server_wait(Some(u64::MAX), 5000), Duration::from_secs(310));
        assert_eq!(server_wait(Some(1500), 5000), Duration::from_millis(1500));
        assert_eq!(server_wait(None, 5000), Duration::from_millis(5000));
        // adding it to a clock never overflows
        assert!(
            Instant::now()
                .checked_add(server_wait(Some(u64::MAX), 0))
                .is_some()
        );
    }

    #[test]
    fn after_child_exit_reports_a_busy_winner_instead_of_a_plain_exit() {
        let poll = Duration::from_millis(1);
        let r = after_child_exit(Duration::ZERO, poll, || Ok(Look::Busy));
        let Err(Fail(1, m)) = r else {
            panic!("busy at the deadline is an error");
        };
        assert!(m.contains("not answering"), "{m}");
        assert!(
            after_child_exit(Duration::ZERO, poll, || Ok(Look::Free))
                .unwrap()
                .is_none()
        );
        let r = after_child_exit(Duration::ZERO, poll, || Ok(Look::Healthy(fake_run(3))));
        assert_eq!(r.unwrap().unwrap().pid, Some(3));
    }

    #[test]
    fn rotate_log_replaces_the_old_backup() {
        let d = dir("rot");
        std::fs::write(d.join("kaanal.log"), "second").unwrap();
        std::fs::write(d.join("kaanal.log.1"), "first").unwrap();
        let log = rotate_log(&d).unwrap();
        assert_eq!(log, d.join("kaanal.log"));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
        assert_eq!(
            std::fs::read_to_string(d.join("kaanal.log.1")).unwrap(),
            "second"
        );
        // no previous log: just a fresh one
        let d = dir("rot-none");
        assert!(rotate_log(&d).unwrap().exists());
        assert!(!d.join("kaanal.log.1").exists());
    }

    #[test]
    fn tail_log_returns_the_last_n_cleaned_lines() {
        let d = dir("tail");
        let f = d.join("l");
        std::fs::write(&f, b"a\nb\x1b[31m\nc\xff\nd\n").unwrap();
        assert_eq!(tail_log(&f, 3), "b?[31m\nc\u{fffd}\nd");
        assert_eq!(tail_log(&f, 99).lines().count(), 4);
        assert_eq!(tail_log(&d.join("missing"), 5), "");
    }

    #[test]
    fn wait_ready_times_out_with_a_fake_poll() {
        let mut n = 0;
        let r = poll_until(Duration::from_millis(40), Duration::from_millis(5), || {
            n += 1;
            Step::Pending
        });
        assert_eq!(r.unwrap_err(), ReadyError::TimedOut);
        assert!(n >= 2);
        let r = poll_until(Duration::from_secs(60), Duration::from_secs(60), || {
            Step::Exited
        });
        assert_eq!(r.unwrap_err(), ReadyError::ChildExited);
        let mut left = 2;
        let r = poll_until(Duration::from_secs(60), Duration::from_millis(1), || {
            left -= 1;
            if left == 0 {
                Step::Ready(Run {
                    addr: "127.0.0.1:1".parse().unwrap(),
                    token: "t".into(),
                    pid: None,
                    startup: Startup::default(),
                })
            } else {
                Step::Pending
            }
        });
        assert_eq!(r.unwrap().token, "t");
    }
    use std::fs::File;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch dir removed on drop.
    struct Tmp(std::path::PathBuf);

    impl std::ops::Deref for Tmp {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Tmp {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "kaanal-life-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    fn hold(d: &Path) -> File {
        let f = File::create(d.join("lock")).unwrap();
        f.try_lock().unwrap();
        f
    }

    fn write_run(d: &Path, token: &str) {
        std::fs::write(
            d.join("run.json"),
            format!(r#"{{"host":"127.0.0.1","port":4000,"token":"{token}","pid":77}}"#),
        )
        .unwrap();
    }

    const SHORT: Duration = Duration::from_millis(300);

    fn code(r: Result<Run, Fail>) -> (u8, String) {
        match r {
            Err(Fail(c, m)) => (c, m),
            Ok(_) => panic!("expected a failure"),
        }
    }

    #[test]
    fn lock_probe_is_free_without_a_lock_file() {
        assert_eq!(lock_probe(&dir("nolock")).unwrap(), Liveness::Free);
    }

    #[test]
    fn lock_probe_is_running_while_another_handle_holds_it() {
        let d = dir("held");
        let _h = hold(&d);
        assert_eq!(lock_probe(&d).unwrap(), Liveness::Running);
    }

    #[test]
    fn lock_probe_is_free_after_the_holder_drops() {
        let d = dir("dropped");
        drop(hold(&d));
        assert_eq!(lock_probe(&d).unwrap(), Liveness::Free);
    }

    #[test]
    fn lock_probe_reports_an_error_instead_of_free() {
        // `lock` is a directory: opening it for locking fails, and that must not read as "stale".
        let d = dir("lockdir");
        std::fs::create_dir(d.join("lock")).unwrap();
        let Err(Fail(c, _)) = lock_probe(&d) else {
            panic!("an unusable lock must be an error");
        };
        assert_eq!(c, 1);
    }

    #[test]
    fn read_run_wait_sees_a_file_that_appears_late() {
        let d = dir("late");
        let d2 = d.to_path_buf();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            write_run(&d2, "tok");
        });
        let run = read_run_wait(&d, Duration::from_secs(10)).expect("file appears");
        assert_eq!(run.token, "tok");
        t.join().unwrap();
    }

    #[test]
    fn read_run_wait_gives_up_after_the_budget() {
        assert!(read_run_wait(&dir("never"), SHORT).is_none());
    }

    #[test]
    fn remove_stale_keeps_a_file_with_a_different_token() {
        let d = dir("tok");
        write_run(&d, "new");
        remove_stale(&d, "old");
        assert!(d.join("run.json").exists());
        remove_stale(&d, "new");
        assert!(!d.join("run.json").exists());
    }

    #[test]
    fn require_instance_covers_every_row() {
        // free + file: stale, exit 2, file removed
        let d = dir("row-stale");
        write_run(&d, "t");
        let (c, m) = code(require_within(&d, SHORT));
        assert_eq!(c, 2);
        assert!(m.contains("removed stale state"), "{m}");
        assert!(!d.join("run.json").exists());
        // free + none: not running
        let (c, m) = code(require_within(&dir("row-none"), SHORT));
        assert_eq!(c, 2);
        assert!(m.contains("no .kaanal/run.json"), "{m}");
        // held + file: ok, and the file is never touched
        let d = dir("row-run");
        let _h = hold(&d);
        write_run(&d, "t");
        let run = require_within(&d, SHORT).unwrap();
        assert_eq!((run.token.as_str(), run.pid), ("t", Some(77)));
        assert!(d.join("run.json").exists());
        // held + none: starting, exit 1 after the budget
        let d = dir("row-start");
        let _h = hold(&d);
        let (c, m) = code(require_within(&d, SHORT));
        assert_eq!(c, 1);
        assert!(m.contains("starting or its state file is missing"), "{m}");
    }

    #[test]
    fn unreadable_run_json_with_a_free_lock_is_removed_and_reported() {
        let d = dir("corrupt");
        std::fs::write(d.join("run.json"), "{ not json").unwrap();
        let (c, m) = code(require_within(&d, SHORT));
        assert_eq!(c, 2);
        assert!(m.contains("removed unreadable state"), "{m}");
        assert!(!d.join("run.json").exists());
    }

    #[test]
    fn run_json_without_a_lock_file_is_stale_and_guarded() {
        let d = dir("nolock-file");
        write_run(&d, "t");
        let (c, m) = code(require_within(&d, SHORT));
        assert_eq!(c, 2);
        assert!(m.contains("removed stale state"), "{m}");
        assert!(!d.join("run.json").exists());
        assert!(d.join("lock").exists(), "the guard needed a lock file");
    }

    #[test]
    fn a_free_probe_holds_the_shared_lock_until_dropped() {
        let d = dir("guard");
        std::fs::write(d.join("lock"), "").unwrap();
        let Probe::Free(Some(g)) = probe(&d).unwrap() else {
            panic!("free with a guard");
        };
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(d.join("lock"))
            .unwrap();
        assert!(matches!(
            f.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(g);
        assert!(f.try_lock().is_ok());
    }

    #[test]
    fn transport_failure_removes_stale_state_when_the_lock_is_free() {
        let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let d = dir("tf-stale");
        write_run(&d, "t");
        let Fail(c, _) = transport_failure(&d, addr, "refused");
        assert_eq!(c, 2);
        assert!(!d.join("run.json").exists());
    }

    #[test]
    fn transport_failure_uses_the_lock() {
        let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let d = dir("tf-free");
        let Fail(c, m) = transport_failure(&d, addr, "refused");
        assert_eq!(c, 2);
        assert!(m.contains("not running on 127.0.0.1:4000: refused"), "{m}");
        let d = dir("tf-held");
        let _h = hold(&d);
        write_run(&d, "t");
        let Fail(c, m) = transport_failure(&d, addr, "timed out");
        assert_eq!(c, 1);
        assert!(
            m.contains("running (pid 77)") && m.contains("timed out"),
            "{m}"
        );
    }

    #[test]
    fn parse_tasklist_csv_reads_the_image_name() {
        let ok = "\"kaanal.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n";
        assert_eq!(parse_tasklist_csv(ok).as_deref(), Some("kaanal.exe"));
        let other = "\r\n\"ping.exe\",\"88\",\"Services\",\"0\",\"4,000 K\"\r\n";
        assert_eq!(parse_tasklist_csv(other).as_deref(), Some("ping.exe"));
        let none = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert_eq!(parse_tasklist_csv(none), None);
        assert_eq!(parse_tasklist_csv(""), None);
    }

    #[test]
    fn parse_ps_comm_trims_and_takes_the_basename() {
        assert_eq!(parse_ps_comm("kaanal\n").as_deref(), Some("kaanal"));
        assert_eq!(parse_ps_comm("  sleep \n").as_deref(), Some("sleep"));
        assert_eq!(
            parse_ps_comm("/usr/local/bin/kaanal\n").as_deref(),
            Some("kaanal")
        );
        assert_eq!(parse_ps_comm("\n"), None);
    }

    #[test]
    fn verify_pid_accepts_this_executable_and_the_standard_name_only() {
        // The unit-test binary is `kaanal-<hash>`: it is kaanal only because it is current_exe.
        assert_eq!(verify_pid(std::process::id()), PidCheck::IsKaanal);
        assert!(is_kaanal_image("KAANAL.EXE", "kaanal.exe", "x", None));
        assert!(is_kaanal_image("kaanal", "kaanal", "", None));
        assert!(is_kaanal_image(
            "My-Kaanal.exe",
            "kaanal.exe",
            "my-kaanal",
            None
        ));
        assert!(!is_kaanal_image(
            "kaanal-1a2b.exe",
            "kaanal.exe",
            "kaanal",
            None
        ));
        assert!(!is_kaanal_image("ping.exe", "kaanal.exe", "", None));
    }

    #[test]
    fn a_truncated_comm_matches_a_longer_own_stem_by_prefix() {
        let own = "kaanal-0123456789abcdef";
        // Linux comm: first 15 characters
        assert!(is_kaanal_image("kaanal-01234567", "kaanal", own, Some(15)));
        assert!(is_kaanal_image("KAANAL-01234567", "kaanal", own, Some(15)));
        // exact matching elsewhere
        assert!(!is_kaanal_image("kaanal-01234567", "kaanal", own, None));
        // another binary with a different prefix, or a shorter one, still does not match
        assert!(!is_kaanal_image("kaanal-99999999", "kaanal", own, Some(15)));
        assert!(!is_kaanal_image("kaanal-0123456", "kaanal", own, Some(15)));
    }

    #[test]
    fn image_names_with_multibyte_text_do_not_panic() {
        assert!(!is_kaanal_image("日本語", "kaanal.exe", "", None));
        assert!(!is_kaanal_image("a日本語.exe", "kaanal.exe", "", Some(15)));
        assert!(is_kaanal_image("日本語.EXE", "kaanal", "日本語", None));
    }

    #[test]
    fn a_failing_lock_probe_is_an_error_not_a_stop() {
        let d = dir("fs-probe-err");
        std::fs::create_dir(d.join("lock")).unwrap(); // unusable lock
        write_run(&d, "t");
        let r = force_stop(
            &d,
            "t",
            77,
            0,
            |_| panic!("no verify"),
            |_| panic!("no kill"),
        );
        let Err(Fail(c, _)) = r else {
            panic!("a probe error must not read as stopped");
        };
        assert_eq!(c, 1);
    }

    #[test]
    fn force_stop_refuses_to_stop_itself() {
        let d = dir("fs-self");
        let _h = hold(&d);
        write_run(&d, "t");
        let r = force_stop(
            &d,
            "t",
            77,
            77,
            |_| panic!("no verify"),
            |_| panic!("no kill"),
        );
        let Err(Fail(1, m)) = r else {
            panic!("must refuse");
        };
        assert!(m.contains("refusing to stop myself"), "{m}");
    }

    #[test]
    fn force_stop_reports_stopped_when_the_instance_vanishes_before_verifying() {
        let d = dir("fs-vanish");
        let h = std::cell::Cell::new(Some(hold(&d)));
        write_run(&d, "t");
        // verify cannot tell, but by then the lock is gone: a stop, not a failure
        let r = force_stop(
            &d,
            "t",
            77,
            0,
            |_| {
                h.take();
                PidCheck::Unknown("tasklist failed".into())
            },
            |_| panic!("must not kill"),
        );
        assert_eq!(r.unwrap(), Forced::Stopped);
    }

    #[test]
    fn force_stop_refuses_unknown_and_other_while_the_lock_is_held() {
        let d = dir("fs-refuse");
        let _h = hold(&d);
        write_run(&d, "t");
        for check in [
            PidCheck::Unknown("x".into()),
            PidCheck::Other("ping.exe".into()),
        ] {
            let c = std::cell::Cell::new(Some(check));
            let r = force_stop(&d, "t", 77, 0, |_| c.take().unwrap(), |_| panic!("no kill"));
            assert!(r.is_err());
        }
    }

    #[test]
    fn force_stop_does_not_kill_when_the_token_changes_before_the_kill() {
        let d = dir("fs-changed");
        let _h = hold(&d);
        write_run(&d, "old");
        let killed = std::cell::Cell::new(false);
        let r = force_stop(
            &d,
            "old",
            77,
            0,
            |_| {
                write_run(&d, "new"); // a new instance took over after the first check
                PidCheck::IsKaanal
            },
            |_| {
                killed.set(true);
                Ok(())
            },
        );
        let Err(Fail(c, m)) = r else {
            panic!("must refuse");
        };
        assert_eq!(c, 1);
        assert!(m.contains("changed while stopping"), "{m}");
        assert!(!killed.get());
    }

    #[test]
    fn force_stop_kills_a_verified_unchanged_instance() {
        let d = dir("fs-kill");
        let h = std::cell::Cell::new(Some(hold(&d)));
        write_run(&d, "t");
        let r = force_stop(
            &d,
            "t",
            77,
            0,
            |_| PidCheck::IsKaanal,
            |pid| {
                assert_eq!(pid, 77);
                h.take(); // the process dies, the OS drops its lock
                Ok(())
            },
        );
        assert_eq!(r.unwrap(), Forced::Killed);
        assert!(!d.join("run.json").exists());
    }

    #[test]
    fn run_or_gone_turns_a_vanished_instance_into_gone() {
        let d = dir("rog");
        let h = hold(&d);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(h);
        });
        let r = run_or_gone(&d, Duration::from_secs(5)).unwrap();
        assert!(matches!(r, Found::Gone(_)));
        t.join().unwrap();
        let d = dir("rog-held");
        let _h = hold(&d);
        assert!(run_or_gone(&d, SHORT).is_err());
    }

    #[test]
    fn wait_lock_free_times_out_while_held_and_sees_a_release() {
        let d = dir("wait");
        let h = hold(&d);
        let t = Instant::now();
        assert!(!wait_lock_free(&d, Duration::from_millis(300)));
        assert!(t.elapsed() >= Duration::from_millis(300));
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(h);
        });
        assert!(wait_lock_free(&d, Duration::from_secs(5)));
        t.join().unwrap();
    }

    #[test]
    fn clear_stale_reports_each_row_and_leaves_a_held_lock_alone() {
        let d = dir("cs-none");
        assert_eq!(clear_stale(&d).unwrap().unwrap(), "kaanal is not running");
        write_run(&d, "t");
        let m = clear_stale(&d).unwrap().unwrap();
        assert!(m.contains("removed stale state from pid 77"), "{m}");
        assert!(!d.join("run.json").exists());
        let d = dir("cs-held");
        let _h = hold(&d);
        write_run(&d, "t");
        assert!(clear_stale(&d).unwrap().is_none());
        assert!(d.join("run.json").exists());
    }

    #[test]
    fn run_json_without_startup_still_parses() {
        let d = dir("nostartup");
        write_run(&d, "t");
        let run = read_run(&d).unwrap();
        assert_eq!(run.startup, Startup::default());
        std::fs::write(
            d.join("run.json"),
            r#"{"port":4000,"token":"t","startup":{"auth":"a","seed":5,"warnings":["w"]}}"#,
        )
        .unwrap();
        let run = read_run(&d).unwrap();
        assert_eq!(run.addr.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(run.startup.seed, Some(5));
        assert_eq!(run.startup.warnings, ["w"]);
    }
}
