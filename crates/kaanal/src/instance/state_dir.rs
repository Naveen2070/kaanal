//! The `.kaanal/` state directory: a private folder, the one-instance lock and `run.json`.

use crate::{
    Duration, Fail, IpAddr, Ipv4Addr, Ipv6Addr, LOCK_FILE, Path, RUN_FILE, SocketAddr, Value, err,
    json, lifecycle,
};

/// **Private.** Creates `dir` (and missing parents) readable only by the current user, before
/// any secret is written into it (R5.8): `.kaanal`, and the signing-key directory (spec X16,
/// on every run, P16). Unix: mode 0700, or an error. Windows: the folder's ACL is replaced by
/// one grant for the current user (inheritance removed), so files created inside never exist
/// with a wider ACL; if that fails we warn and carry on: the token still protects the control API.
pub(crate) fn private_dir(dir: &Path) -> Result<(), Fail> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        b.mode(0o700);
        b.create(dir)
            .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)))
            .map_err(|e| err(format!("{}: {e}", dir.display())))?;
    }
    #[cfg(not(unix))]
    b.create(dir)
        .map_err(|e| err(format!("{}: {e}", dir.display())))?;
    #[cfg(windows)]
    if let Err(e) = restrict_to_current_user(dir) {
        eprintln!(
            "warning: could not restrict {} to the current user: {e}",
            dir.display()
        );
    }
    Ok(())
}

/// **Private.** Replaces the directory's ACL with one grant for the current user and removes
/// explicit grants to Everyone, Users and Authenticated Users, using System32 binaries by
/// full path (a planted earlier-PATH exe is never run).
#[cfg(windows)]
pub(crate) fn restrict_to_current_user(dir: &Path) -> Result<(), String> {
    let root = std::env::var("SystemRoot").map_err(|_| "SystemRoot is not set")?;
    // Full paths, so an icacls.exe or whoami.exe planted earlier on PATH is never run.
    let who = lifecycle::os_command(format!(r"{root}\System32\whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|e| format!("whoami: {e}"))?;
    // Output looks like: "host\user","S-1-5-21-..."
    let text = String::from_utf8_lossy(&who.stdout);
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .map(|s| s.trim_matches('"'))
        .filter(|s| s.starts_with("S-1-"))
        .ok_or("could not read the current user's SID")?;
    let icacls = format!(r"{root}\System32\icacls.exe");
    let out = lifecycle::os_command(&icacls)
        .arg(dir)
        .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(OI)(CI)F")])
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    // `/inheritance:r` drops inherited entries only. Remove explicit grants to the wide
    // well-known groups (Everyone, Users, Authenticated Users) too, on the folder and the
    // files already in it. Not `/reset`: it would briefly widen a running instance's files.
    let out = lifecycle::os_command(&icacls)
        .arg(dir)
        .args([
            "/remove:g",
            "*S-1-1-0",
            "*S-1-5-32-545",
            "*S-1-5-11",
            "/T",
            "/C",
            "/Q",
        ])
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// **Private.** Exclusive advisory lock on `.kaanal/lock` (std `File::try_lock`, Rust 1.89+).
pub(crate) fn lock_instance() -> Result<std::fs::File, Fail> {
    lock_at(Path::new(LOCK_FILE), 10, Duration::from_millis(20))
}

/// **Private.** [`lock_instance`] on `path`. A status probe holds a shared lock for an instant, so
/// `WouldBlock` is retried `tries` times, `gap` apart, before it is called "already running"; a live
/// instance holds the lock forever, so a true "already running" costs about `tries * gap`.
pub(crate) fn lock_at(path: &Path, tries: u32, gap: Duration) -> Result<std::fs::File, Fail> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    debug_assert!(tries > 0, "at least one attempt");
    for left in (0..tries.max(1)).rev() {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::WouldBlock) if left > 0 => std::thread::sleep(gap),
            Err(std::fs::TryLockError::WouldBlock) => break,
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(err(format!("{}: {e}", path.display())));
            }
        }
    }
    let port = path
        .parent()
        .and_then(lifecycle::read_run)
        .map_or(String::new(), |r| format!(" on port {}", r.addr.port()));
    Err(err(format!(
        "kaanal is already running{port} (this directory's {LOCK_FILE} is held)"
    )))
}

/// **Private.** Writes `{host, port, token, pid, startup}` via temp file + rename, so a concurrent
/// `status` never reads half a file. `host` is where the CLI connects: the bound address,
/// or loopback when bound to all interfaces.
pub(crate) fn write_run_file(bound: SocketAddr, token: &str, startup: &Value) -> Result<(), Fail> {
    use std::io::Write;
    let host = match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let body = json!({
        "host": host, "port": bound.port(), "token": token, "pid": std::process::id(),
        "startup": startup,
    })
    .to_string();
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    // Write a temp file, then rename over the real one, so a concurrent `kaanal status` never reads half a file.
    let tmp = format!("{RUN_FILE}.tmp");
    opts.open(&tmp)
        .and_then(|mut f| f.write_all(body.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, RUN_FILE))
        .map_err(|e| err(format!("{RUN_FILE}: {e}")))
}
