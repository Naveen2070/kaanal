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
mod liveness;
mod pid;
mod ready;
mod spawn;
mod stop;
#[cfg(test)]
mod tests;

pub use liveness::{
    Liveness, Run, clear_stale, lock_probe, read_run, require_instance, transport_failure,
    wait_lock_free,
};
#[cfg(windows)]
pub use pid::os_command;
pub use pid::{PidCheck, kill_pid, verify_pid};
pub use ready::{
    Existing, Look, ReadyError, Settled, after_child_exit, existing_instance, healthy, run_of,
    server_wait, settle_unready, tail_log, wait_ready,
};
pub use spawn::{rotate_log, spawn_detached};
pub use stop::{Forced, Found, force_stop, ignore_hangup, run_or_gone, stop_signal};

// What only the unit tests (`tests.rs`) reach by `super::`.
#[cfg(test)]
use liveness::{Probe, Startup, probe, read_run_wait, remove_stale, require_within};
#[cfg(test)]
use pid::{is_kaanal_image, parse_ps_comm, parse_tasklist_csv};
#[cfg(test)]
use ready::{Step, poll_until};

/// **Private.** How long a command waits for `run.json` while the lock is held (M17).
pub const STARTUP_WAIT: Duration = Duration::from_secs(5);
