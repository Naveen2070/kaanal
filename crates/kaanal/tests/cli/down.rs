//! `kaanal down`.

use crate::common::*;

use std::path::Path;

use std::process::{Child, Command, Output, Stdio};

use std::time::{Duration, Instant};

impl std::ops::Deref for Helper {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Helper {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn down_in(dir: &Path, args: &[&str]) -> Output {
    bin()
        .arg("down")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A TCP port nothing listens on.
pub(crate) fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn down_stops_a_running_up_and_status_then_exits_2() {
    let mut up = Up::start("down-stops");
    let out = up.run(&["down"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "stopped");
    assert_eq!(wait_exit(&mut up.child, 20), Some(0));
    assert!(!up.dir.join(".kaanal/run.json").exists());
    assert_eq!(up.run(&["status"]).status.code(), Some(2));
}

#[test]
fn down_twice_exits_0_both_times() {
    let up = Up::start("down-twice");
    for _ in 0..2 {
        let out = up.run(&["down"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    }
    assert!(stdout(&up.run(&["down"])).contains("not running"));
}

#[test]
fn down_with_nothing_running_exits_0() {
    let dir = scratch("down-nothing");
    let out = down_in(&dir, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "kaanal is not running");
}

#[test]
fn down_removes_stale_state_and_exits_0() {
    let dir = scratch("down-stale");
    let mut child = Helper(spawn_up(&dir));
    wait_for_pid(&dir, &mut child);
    let pid = child.id();
    child.kill().unwrap();
    child.wait().unwrap();
    let out = down_in(&dir, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains(&format!("removed stale state from pid {pid}")),
        "{}",
        stdout(&out)
    );
    assert!(!dir.join(".kaanal/run.json").exists());
}

#[test]
fn down_kills_after_a_wrong_token() {
    let mut up = Up::start("down-wrong-token");
    let mut run = run_json(&up.dir);
    let real = run["token"].as_str().unwrap().to_string();
    run["token"] = serde_json::json!("not-the-token");
    std::fs::write(up.dir.join(".kaanal/run.json"), run.to_string()).unwrap();
    let t = Instant::now();
    let out = up.run(&["down", "--timeout", "10s"]);
    // A 401 means the API is up but unusable: straight to the kill, no drain wait (10s + 2s).
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let pid = up.child.id();
    assert_eq!(
        stdout(&out).trim(),
        format!("stopped (killed pid {pid} after 10s)")
    );
    assert!(!stdout(&out).contains(&real) && !stderr(&out).contains(&real));
    up.child.wait().unwrap();
    assert_eq!(up.run(&["status"]).status.code(), Some(2));
}

#[test]
fn down_refuses_to_kill_a_non_kaanal_pid() {
    let dir = scratch("down-other");
    let _lock = hold_lock(&dir);
    #[cfg(windows)]
    let helper = Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    #[cfg(not(windows))]
    let helper = Command::new("sleep").arg("30").spawn().unwrap();
    let mut helper = Helper(helper);
    let pid = helper.0.id();
    std::fs::write(
        dir.join(".kaanal/run.json"),
        format!(r#"{{"port":{},"token":"x","pid":{pid}}}"#, closed_port()),
    )
    .unwrap();
    let out = down_in(&dir, &["--timeout", "1s"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let e = stderr(&out);
    assert!(
        e.contains(&format!("pid {pid}")) && e.contains("not a kaanal process"),
        "{e}"
    );
    assert!(
        helper.0.try_wait().unwrap().is_none(),
        "the helper must still be alive"
    );
}

#[test]
fn down_cuts_a_hanging_request_at_the_budget() {
    let up = Up::start("down-hang");
    for node in ["nginx", "app", "db"] {
        ok(&up, &["chaos", "set", node, "--latency", "30s"]);
    }
    let port = run_json(&up.dir)["port"].as_u64().unwrap();
    let hang = std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let _ = agent.get(&format!("http://127.0.0.1:{port}/orders")).call();
    });
    std::thread::sleep(Duration::from_millis(400));
    let t = Instant::now();
    let out = up.run(&["down", "--timeout", "1s"]);
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "stopped");
    hang.join().unwrap();
}

#[test]
fn concurrent_downs_both_succeed() {
    let up = Up::start("down-concurrent");
    let spawn = || {
        bin()
            .arg("down")
            .current_dir(&up.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let (a, b) = (spawn(), spawn());
    for out in [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()] {
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(!stderr(&out).contains("panicked"), "{}", stderr(&out));
    }
}

#[test]
fn down_during_startup_window_waits_then_fails_clearly() {
    let dir = scratch("down-starting");
    let _lock = hold_lock(&dir);
    let t = Instant::now();
    let out = down_in(&dir, &[]);
    assert!(t.elapsed() >= Duration::from_secs(4), "{:?}", t.elapsed());
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stderr(&out).contains("starting"), "{}", stderr(&out));
}

#[test]
fn bad_timeout_is_usage_64() {
    for t in ["soon", "5", "0s", "301s"] {
        assert_64(&["down", "--timeout", t]);
    }
}

/// Not run on Windows (no SIGTERM there): the Windows stop path is covered by the POST test above.
#[cfg(unix)]
#[test]
fn sigterm_stops_up_gracefully() {
    // Signal the moment run.json appears: the handlers must already be registered, so every
    // round drains and exits 0 (a late registration dies from the signal's default action).
    for round in 0..20 {
        let dir = scratch("sigterm"); // scratch() wipes and recreates it each round
        let mut child = Helper(spawn_up(&dir));
        wait_for_pid(&dir, &mut child);
        let st = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(st.success());
        assert_eq!(wait_exit(&mut child, 20), Some(0), "round {round}");
        assert!(!dir.join(".kaanal/run.json").exists());
    }
}

// ---- `kaanal up -d` ----
