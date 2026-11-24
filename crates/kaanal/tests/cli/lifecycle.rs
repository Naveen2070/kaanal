//! Stale state, the startup window and the stop paths.

use crate::common::*;

use std::io::{BufRead, BufReader};

use std::process::Stdio;

#[test]
fn up_writes_a_token_free_startup_object_to_run_json() {
    let up = Up::start("startup");
    let run = run_json(&up.dir);
    let token = run["token"].as_str().unwrap();
    assert_ne!(token, "");
    let startup = &run["startup"];
    assert!(startup["warnings"].is_array(), "{run}");
    assert!(!startup.to_string().contains(token), "{startup}");
}

#[test]
fn post_shutdown_stops_a_foreground_up_and_removes_run_json() {
    let mut up = Up::start("post-shutdown");
    let run = run_json(&up.dir);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    let port = run["port"].as_u64().unwrap();
    let resp = agent
        .post(&format!("http://127.0.0.1:{port}/__kaanal/shutdown"))
        .header("Host", &format!("localhost:{port}"))
        .header(
            "Authorization",
            &format!("Bearer {}", run["token"].as_str().unwrap()),
        )
        .send_empty()
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    assert_eq!(wait_exit(&mut up.child, 20), Some(0));
    assert!(!up.dir.join(".kaanal/run.json").exists());
}

#[test]
fn a_killed_up_leaves_stale_state_that_status_reports_and_removes() {
    let dir = scratch("killed-stale");
    let mut child = Helper(spawn_up(&dir));
    wait_for_pid(&dir, &mut child);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        dir.join(".kaanal/run.json").exists(),
        "a kill leaves the file"
    );

    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("stale state"), "{}", stderr(&out));
    assert!(!dir.join(".kaanal/run.json").exists());
}

#[test]
fn up_over_stale_state_starts_normally() {
    let dir = scratch("over-stale");
    std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
    std::fs::write(
        dir.join(".kaanal/run.json"),
        r#"{"port":9,"token":"stale","pid":1}"#,
    )
    .unwrap();
    let mut child = Helper(spawn_up(&dir));
    wait_for_pid(&dir, &mut child);
    assert_ne!(run_json(&dir)["token"], "stale");
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    drop(child);
}

#[test]
fn detached_flag_prints_ui_url_without_token() {
    let dir = scratch("detached");
    let mut child = bin()
        .args(["up", "--detached", "--port", "0", "-c"])
        .arg(fixture())
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ui = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(Result::unwrap)
        .find(|l| l.starts_with("UI:"));
    stop(child);
    let ui = ui.expect("up prints a UI: line");
    // a loaded project opens on Studio's Admin tab; the detached child's output carries no token
    assert!(ui.ends_with("/__kaanal/studio?mode=admin"), "{ui}");
    assert!(!ui.contains("token"), "{ui}");
}

#[test]
fn foreground_up_keeps_the_token_in_the_ui_url() {
    let dir = scratch("fg-ui");
    let mut child = bin()
        .args(["up", "--port", "0", "-c"])
        .arg(fixture())
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ui = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(Result::unwrap)
        .find(|l| l.starts_with("UI:"));
    stop(child);
    assert!(ui.unwrap().contains("/__kaanal/studio?mode=admin#token="));
}

#[test]
fn held_lock_without_run_json_is_starting_exit_1() {
    let dir = scratch("starting");
    let _lock = hold_lock(&dir);
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("starting or its state file"),
        "{}",
        stderr(&out)
    );
}

// Stops and joins the listener thread even if an assert below fails first.
pub(crate) struct StopListener(
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    u16,
    Option<std::thread::JoinHandle<()>>,
);
impl Drop for StopListener {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.1));
        if let Some(t) = self.2.take() {
            let _ = t.join();
        }
    }
}

#[test]
fn held_lock_with_an_unanswering_api_is_exit_1_not_2() {
    let dir = scratch("wedged");
    let _lock = hold_lock(&dir);
    // A server that accepts and drops every connection: alive, never answering, no port to race for.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&done);
    let server = std::thread::spawn(move || {
        while let Ok((conn, _)) = listener.accept() {
            drop(conn);
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
        }
    });
    let _stop = StopListener(done, port, Some(server));
    std::fs::write(
        dir.join(".kaanal/run.json"),
        format!(r#"{{"port":{port},"token":"x","pid":4242}}"#),
    )
    .unwrap();
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("did not answer"), "{}", stderr(&out));
    assert!(
        dir.join(".kaanal/run.json").exists(),
        "a held lock keeps the file"
    );
}

// ---- kaanal down (C2a task 4) ----
