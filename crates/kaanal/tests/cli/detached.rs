//! `kaanal up -d`.

use crate::common::*;

use std::path::{Path, PathBuf};

use std::process::{Command, Output, Stdio};

use std::time::{Duration, Instant};

/// Stops whatever daemon lives in `dir` when dropped, even after a failed assertion: `down`, then
/// a hard kill of the pid in run.json if one is left.
pub(crate) struct Daemon(pub(crate) PathBuf);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = bin()
            .args(["down", "--timeout", "2s"])
            .current_dir(&self.0)
            .output();
        if let Some(pid) = run_json_pid(&self.0) {
            let _ = if cfg!(windows) {
                Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output()
            } else {
                Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .output()
            };
        }
    }
}

/// `kaanal up -d --port 0 -c <cfg>` in `dir`.
pub(crate) fn up_d_cmd(dir: &Path, cfg: &Path) -> Command {
    let mut c = bin();
    c.args(["up", "-d", "--port", "0", "-c"])
        .arg(cfg)
        .current_dir(dir);
    c
}

pub(crate) fn mocks_port(out: &Output) -> u16 {
    stdout(out)
        .lines()
        .find_map(|l| l.strip_prefix("Mocks: http://localhost:"))
        .unwrap_or_else(|| panic!("no Mocks line: {} / {}", stdout(out), stderr(out)))
        .parse()
        .unwrap()
}

pub(crate) fn run_token(dir: &Path) -> String {
    let s = std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&s).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn piped_up_d_returns_while_the_daemon_keeps_running() {
    let dir = scratch("upd-pipe");
    let _d = Daemon(dir.clone());
    let mut child = up_d_cmd(&dir, &fixture())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let rx = drain_pipes(&mut child);
    for _ in 0..2 {
        rx.recv_timeout(Duration::from_secs(20))
            .expect("EOF on stdout/stderr within 20 s: the daemon inherited the caller's pipes");
    }
    assert_eq!(child.wait().unwrap().code(), Some(0));
    let st = kaanal(&dir, &["status"]);
    assert_eq!(st.status.code(), Some(0), "{}", stderr(&st));
    let down = kaanal(&dir, &["down"]);
    assert_eq!(down.status.code(), Some(0), "{}", stderr(&down));
}

#[test]
fn up_d_returns_after_health_and_run_json_has_a_live_port() {
    let dir = scratch("upd-live");
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let port = mocks_port(&out);
    let text = stdout(&out);
    assert!(
        text.contains(&format!("#token={}", run_token(&dir))),
        "{text}"
    );
    assert!(
        text.contains("Log: ") && text.contains("Stop: kaanal down"),
        "{text}"
    );
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    assert_eq!(run["port"].as_u64(), Some(u64::from(port)));
    assert_eq!(health(port), 200);
}

#[test]
fn up_d_log_exists_and_holds_no_token() {
    let dir = scratch("upd-log");
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let token = run_token(&dir);
    let log = std::fs::read_to_string(dir.join(".kaanal/kaanal.log")).unwrap();
    assert!(log.contains("Mocks: "), "{log}");
    assert!(!log.contains(&token) && !log.contains("token="), "{log}");
}

#[test]
fn second_up_d_exits_0_with_already_running_and_the_same_port() {
    let dir = scratch("upd-twice");
    let _d = Daemon(dir.clone());
    let first = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let second = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    assert_eq!(mocks_port(&first), mocks_port(&second));
    let pid = run_json_pid(&dir).unwrap();
    assert!(
        stdout(&second).contains(&format!("already running (pid {pid})")),
        "{}",
        stdout(&second)
    );
}

#[test]
fn up_d_then_down_then_status_exits_2() {
    let dir = scratch("upd-down");
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(kaanal(&dir, &["down"]).status.code(), Some(0));
    assert_eq!(kaanal(&dir, &["status"]).status.code(), Some(2));
}

#[test]
fn up_d_rotates_the_previous_log() {
    let dir = scratch("upd-rotate");
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let first = std::fs::read_to_string(dir.join(".kaanal/kaanal.log")).unwrap();
    assert_ne!(first, "");
    assert_eq!(kaanal(&dir, &["down"]).status.code(), Some(0));
    let out = piped(up_d_cmd(&dir, &fixture()));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let old = std::fs::read_to_string(dir.join(".kaanal/kaanal.log.1")).unwrap();
    assert_eq!(old, first);
}

#[test]
fn up_d_with_a_bad_config_exits_1_and_spawns_nothing() {
    let dir = scratch("upd-bad");
    // Parses, but fails the topology check: only the parent's validation can stop it.
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n  b: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: a, to: b }\n  - { from: b, to: a }\n",
    )
    .unwrap();
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, Path::new("kaanal.yaml")));
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let e = stderr(&out);
    assert!(e.contains("kaanal.yaml") && e.contains("cycle"), "{e}");
    assert!(
        !dir.join(".kaanal").exists(),
        "nothing may be created or spawned"
    );
}

#[test]
fn up_d_with_a_missing_config_exits_1() {
    let dir = scratch("upd-missing");
    let out = piped(up_d_cmd(&dir, Path::new("nope.yaml")));
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("nope.yaml"), "{}", stderr(&out));
    assert!(!dir.join(".kaanal").exists());
}

#[test]
fn up_d_when_the_port_search_is_exhausted_exits_1_with_log_lines() {
    // Hold 21 consecutive ports so the +20 search finds none.
    let (base, _held) = (41_000u16..60_000)
        .step_by(37)
        .find_map(|p| {
            (p..=p + 20)
                .map(|q| std::net::TcpListener::bind(("127.0.0.1", q)))
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .map(|l| (p, l))
        })
        .expect("21 free consecutive ports");
    let dir = scratch("upd-ports");
    let _d = Daemon(dir.clone());
    let mut c = bin();
    c.args(["up", "-d", "--port", &base.to_string(), "-c"])
        .arg(fixture())
        .current_dir(&dir);
    let out = piped(c);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stderr(&out).contains("no free port"), "{}", stderr(&out));
}

#[test]
fn up_d_prints_auth_and_seed_lines() {
    let (dir, cfg) = auth_project("upd-auth", true);
    let mut src = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    src.push_str("determinism: realistic\n");
    std::fs::write(dir.join("kaanal.yaml"), src).unwrap();
    let _d = Daemon(dir.clone());
    let mut c = up_d_cmd(&dir, Path::new("kaanal.yaml"));
    c.env("KAANAL_CONFIG_DIR", &cfg);
    let out = piped(c);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("Auth:  POST http://localhost:") && text.contains("1 user"),
        "{text}"
    );
    assert!(
        text.contains("Seed:  ") && text.contains("determinism: realistic"),
        "{text}"
    );
}

#[test]
fn up_d_works_in_a_path_with_spaces_and_non_ascii() {
    let dir = project("upd sp\u{e4}ce \u{65e5}\u{672c}", "");
    let _d = Daemon(dir.clone());
    let out = piped(up_d_cmd(&dir, Path::new("kaanal.yaml")));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(health(mocks_port(&out)), 200);
}

#[test]
fn simultaneous_up_d_starts_exactly_one_instance() {
    let dir = scratch("upd-race");
    let _d = Daemon(dir.clone());
    let go = |d: PathBuf| std::thread::spawn(move || piped(up_d_cmd(&d, &fixture())));
    let all: Vec<_> = (0..5).map(|_| go(dir.clone())).collect();
    let outs: Vec<Output> = all.into_iter().map(|t| t.join().unwrap()).collect();
    for o in &outs {
        assert_eq!(o.status.code(), Some(0), "{}", stderr(o));
    }
    let port = mocks_port(&outs[0]);
    assert!(
        outs.iter().all(|o| mocks_port(o) == port),
        "one instance, one port"
    );
    assert_eq!(
        outs.iter()
            .filter(|o| stdout(o).contains("already running"))
            .count(),
        4,
        "exactly one start, four already-running"
    );
    assert_eq!(kaanal(&dir, &["status"]).status.code(), Some(0));
    // The live daemon writes to kaanal.log; nobody rotated it away.
    let log = std::fs::read_to_string(dir.join(".kaanal/kaanal.log")).unwrap();
    assert!(
        log.contains(&format!("Mocks: http://localhost:{port}")),
        "{log}"
    );
    assert!(!dir.join(".kaanal/kaanal.log.1").exists());
}

#[test]
fn up_d_that_times_out_leaves_no_process_behind() {
    let (dir, cfg) = auth_project("upd-timeout", true);
    let _d = Daemon(dir.clone());
    let mut c = up_d_cmd(&dir, Path::new("kaanal.yaml"));
    c.env("KAANAL_CONFIG_DIR", &cfg).args(["--wait", "1ms"]);
    let out = piped(c);
    if out.status.code() == Some(0) {
        // A fast machine was ready within (or just after) the 1ms deadline and is kept, so there
        // is nothing to end here; the kill path is unit-tested in `settle_unready`.
        assert_eq!(kaanal(&dir, &["down"]).status.code(), Some(0));
        return;
    }
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("no answer within"),
        "{}",
        stderr(&out)
    );
    // The child was ended, not orphaned: its lock is free again (status says stale or absent).
    assert!(
        kaanal_server_lock_free(&dir),
        "an orphaned kaanal process still holds the lock"
    );
    assert_eq!(kaanal(&dir, &["status"]).status.code(), Some(2));
}

/// Whether nothing holds `.kaanal/lock` (polled for 3 s: the OS frees a killed process's lock a moment later).
pub(crate) fn kaanal_server_lock_free(dir: &Path) -> bool {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let free = match std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join(".kaanal/lock"))
        {
            Ok(f) => f.try_lock().is_ok(),
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if free || Instant::now() >= end {
            return free;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn up_d_warns_about_a_non_loopback_host_in_the_parent() {
    let dir = scratch("upd-host");
    let _d = Daemon(dir.clone());
    let mut c = up_d_cmd(&dir, &fixture());
    c.args(["--host", "0.0.0.0"]);
    let out = piped(c);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let e = stderr(&out);
    assert_eq!(e.matches("warning: binding 0.0.0.0").count(), 1, "{e}");
    assert!(
        e.contains("reachable from your network; control calls still need the token and Host"),
        "{e}"
    );
}

#[test]
fn wait_zero_is_usage_64() {
    let dir = scratch("upd-wait0");
    let mut c = up_d_cmd(&dir, &fixture());
    c.args(["--wait", "0s"]);
    let out = piped(c);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(!dir.join(".kaanal").exists());
}
