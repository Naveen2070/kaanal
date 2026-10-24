//! Telling whether a pid is a `kaanal` process (`tasklist` / `ps` parsing) and killing it.

#[cfg(not(windows))]
use std::path::Path;

use super::Command;

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
pub(super) fn os_tool(name: &str) -> Result<std::path::PathBuf, String> {
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
pub(super) fn run_tool(name: &str, args: &[&str]) -> Result<String, String> {
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
pub(super) fn own_stem() -> String {
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
pub(super) fn is_kaanal_image(
    name: &str,
    want: &str,
    own: &str,
    comm_limit: Option<usize>,
) -> bool {
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
pub(super) const COMM_LIMIT: Option<usize> = if cfg!(target_os = "linux") {
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
