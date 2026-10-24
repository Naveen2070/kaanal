//! Starting the background instance for `up -d`: the log rotation and the detached spawn.

use super::{Child, Command, Fail, File, OsString, Path, PathBuf, Stdio, err};

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
