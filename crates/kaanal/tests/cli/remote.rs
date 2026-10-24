//! `--endpoint` and `--token`, remote `down` and completions.

use crate::common::*;

use std::io::{BufRead, BufReader, Read};

use std::process::Stdio;

use std::time::{Duration, Instant};

/// A port nothing listens on (bound, then released).
pub(crate) fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn endpoint_reaches_an_instance_from_another_directory() {
    let up = Up::start("remote-ok");
    let (port, token) = port_and_token(&up);
    let elsewhere = scratch("remote-ok-elsewhere");
    let url = format!("http://127.0.0.1:{port}/");
    let out = remote(
        &elsewhere,
        &["--endpoint", &url, "status"],
        &[("KAANAL_TOKEN", &token)],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("postgres"), "{}", stdout(&out));
    // The env form works too, and remote mode never creates local state.
    let out = remote(
        &elsewhere,
        &["--json", "pins"],
        &[("KAANAL_ENDPOINT", &url), ("KAANAL_TOKEN", &token)],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!elsewhere.join(".kaanal").exists());
    assert!(!stderr(&out).contains("warning"), "{}", stderr(&out));
}

#[test]
fn endpoint_without_a_token_is_usage_error_64() {
    let dir = scratch("remote-notoken");
    let out = remote(&dir, &["--endpoint", "http://127.0.0.1:1", "status"], &[]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("needs a token"), "{}", stderr(&out));
    // An empty env value is unset.
    let out = remote(
        &dir,
        &["status"],
        &[
            ("KAANAL_ENDPOINT", "http://127.0.0.1:1"),
            ("KAANAL_TOKEN", ""),
        ],
    );
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

#[test]
fn token_flag_without_endpoint_is_64_but_env_token_alone_is_ignored() {
    let up = Up::start("remote-tokflag");
    for tok in ["x", ""] {
        let out = up.run(&["--token", tok, "status"]);
        assert_eq!(out.status.code(), Some(64), "{tok:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("--endpoint"), "{}", stderr(&out));
    }
    // `down` and `up` do not resolve a target, but still refuse the flag; nothing is stopped.
    let cfg = fixture();
    for args in [
        &["--token", "x", "down"][..],
        &["down", "--token", "x"][..],
        &["--token", "x", "up", "-c", cfg.to_str().unwrap()][..],
    ] {
        let out = up.run(args);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("--endpoint"), "{}", stderr(&out));
    }
    ok(&up, &["status"]);
    // A CI shell that exports KAANAL_TOKEN still runs local commands (Review Focus 6).
    let out = remote(&up.dir, &["status"], &[("KAANAL_TOKEN", "unrelated")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // An empty KAANAL_ENDPOINT is unset, so this is local too.
    let out = remote(&up.dir, &["status"], &[("KAANAL_ENDPOINT", "")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn https_endpoint_is_64() {
    let dir = scratch("remote-https");
    for url in [
        "https://localhost:8080",
        "http://localhost:8080/x",
        "localhost:8080",
        "http://localhost",
    ] {
        let out = remote(&dir, &["--endpoint", url, "--token", "t", "status"], &[]);
        assert_eq!(out.status.code(), Some(64), "{url}: {}", stderr(&out));
    }
    let out = remote(
        &dir,
        &[
            "--endpoint",
            "https://localhost:8080",
            "--token",
            "t",
            "status",
        ],
        &[],
    );
    assert!(stderr(&out).contains("plain http"), "{}", stderr(&out));
    // A password in the URL is refused and never echoed.
    let out = remote(
        &dir,
        &[
            "--endpoint",
            "http://u:hunter2@localhost:1",
            "--token",
            "t",
            "status",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(!stderr(&out).contains("hunter2"), "{}", stderr(&out));
}

#[test]
fn up_with_an_endpoint_is_64() {
    let dir = scratch("remote-up");
    let cfg = fixture();
    let cfg = cfg.to_str().unwrap();
    let out = remote(
        &dir,
        &[
            "--endpoint",
            "http://127.0.0.1:1",
            "--token",
            "t",
            "up",
            "-c",
            cfg,
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("--endpoint"), "{}", stderr(&out));
    for args in [&["up", "-c", cfg][..], &["up", "-d", "-c", cfg][..]] {
        let out = remote(&dir, args, &[("KAANAL_ENDPOINT", "http://127.0.0.1:1")]);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("KAANAL_ENDPOINT"), "{}", stderr(&out));
    }
    assert!(!dir.join(".kaanal").exists());
}

#[test]
fn remote_refused_is_exit_1_not_2() {
    let dir = scratch("remote-refused");
    let url = format!("http://127.0.0.1:{}", free_port());
    let out = remote(
        &dir,
        &["--endpoint", &url, "status"],
        &[("KAANAL_TOKEN", "t0ken-x")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains(&format!("cannot reach {url}")), "{err}");
    assert!(!err.contains("t0ken-x"), "{err}");
    let out = remote(
        &dir,
        &["--endpoint", &url, "logs", "-f"],
        &[("KAANAL_TOKEN", "t0ken-x")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("cannot reach"), "{}", stderr(&out));
}

#[test]
fn wrong_token_is_exit_1_token_rejected() {
    let up = Up::start("remote-badtoken");
    let (port, _) = port_and_token(&up);
    let url = format!("http://localhost:{port}");
    let elsewhere = scratch("remote-badtoken-elsewhere");
    for args in [
        &["--endpoint", &url, "status"][..],
        &["--endpoint", &url, "logs", "-f"][..],
    ] {
        let out = remote(&elsewhere, args, &[("KAANAL_TOKEN", "s3cr3t-wrong")]);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(err.contains("token rejected"), "{err}");
        assert!(!err.contains("s3cr3t-wrong"), "{err}");
    }
}

#[test]
fn non_loopback_endpoint_warns_once_on_stderr_only() {
    let dir = scratch("remote-warn");
    // Unroutable: the connect times out (or is refused) after the warning.
    let out = remote(
        &dir,
        &["--endpoint", "http://10.255.255.1:1", "status"],
        &[("KAANAL_TOKEN", "t0ken-y")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    let warning = "warning: sending the token over plain http to 10.255.255.1";
    assert_eq!(err.matches(warning).count(), 1, "{err}");
    assert!(!err.contains("t0ken-y"), "{err}");
    assert!(!stdout(&out).contains("warning"), "{}", stdout(&out));
}

#[test]
fn unresolvable_host_is_exit_1() {
    let dir = scratch("remote-dns");
    let out = remote(
        &dir,
        &["--endpoint", "http://no-such-host.invalid:8080", "status"],
        &[("KAANAL_TOKEN", "t")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("cannot reach"), "{}", stderr(&out));
}

#[test]
fn local_down_ignores_an_unrelated_token_env() {
    let up = Up::start("remote-down-local");
    let out = remote(&up.dir, &["down"], &[("KAANAL_TOKEN", "unrelated")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "stopped");
    assert!(!up.dir.join(".kaanal/run.json").exists());
}

#[test]
fn remote_down_stops_the_instance_without_touching_pids() {
    let mut up = Up::start("remote-down");
    let (port, token) = port_and_token(&up);
    let elsewhere = scratch("remote-down-elsewhere");
    let url = format!("http://127.0.0.1:{port}");
    let out = remote(
        &elsewhere,
        &["--endpoint", &url, "down"],
        &[("KAANAL_TOKEN", &token)],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "stopped");
    assert!(!stderr(&out).contains(&token));
    assert!(!elsewhere.join(".kaanal").exists());
    // The instance drained and exited on its own.
    let deadline = Instant::now() + Duration::from_secs(10);
    while up.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "instance still running");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn remote_down_when_nothing_answers_is_exit_1() {
    let dir = scratch("remote-down-none");
    let url = format!("http://127.0.0.1:{}", free_port());
    let out = remote(
        &dir,
        &["--endpoint", &url, "down"],
        &[("KAANAL_TOKEN", "t0ken-z")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("cannot reach"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("t0ken-z"));
}

/// Reads what is left of a request so closing the socket does not reset the client mid-send.
pub(crate) fn drain_request(conn: &mut std::net::TcpStream) {
    let mut buf = [0u8; 4096];
    conn.set_read_timeout(Some(Duration::from_millis(100))).ok();
    while conn.read(&mut buf).is_ok_and(|n| n > 0) {}
}

#[test]
fn remote_down_that_never_stops_is_exit_1_and_does_not_kill() {
    use std::io::Write as _;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Answers 202 to the shutdown POST and 200 to health, forever.
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut conn = conn;
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]);
            let status = if head.starts_with("POST /__kaanal/shutdown") {
                "202 Accepted"
            } else {
                "200 OK"
            };
            let _ = write!(
                conn,
                "HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
            );
            drain_request(&mut conn);
        }
    });
    let dir = scratch("remote-down-stuck");
    let url = format!("http://127.0.0.1:{port}");
    let out = remote(
        &dir,
        &["--endpoint", &url, "down", "--timeout", "200ms"],
        &[("KAANAL_TOKEN", "t")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("still answering"), "{err}");
    assert!(err.contains("not killing"), "{err}");
    assert!(!dir.join(".kaanal").exists());
}

#[test]
fn remote_down_with_a_silent_health_is_exit_1_not_stopped() {
    use std::io::Write as _;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Accepts every connection; answers the shutdown POST but never answers health.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for mut conn in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap_or(0);
            if String::from_utf8_lossy(&buf[..n]).starts_with("POST") {
                let _ = write!(
                    conn,
                    "HTTP/1.1 202 Accepted\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
                );
                drain_request(&mut conn);
            } else {
                held.push(conn);
            }
        }
    });
    let dir = scratch("remote-down-silent");
    let url = format!("http://127.0.0.1:{port}");
    let started = Instant::now();
    let out = remote(
        &dir,
        &["--endpoint", &url, "down", "--timeout", "200ms"],
        &[("KAANAL_TOKEN", "t")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "");
    assert!(stderr(&out).contains("still answering"), "{}", stderr(&out));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn completions_print_a_script_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = bin().args(["completions", shell]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{shell}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("chaos"), "{shell}: no chaos in script");
    }
}

#[test]
fn completions_need_no_instance_and_ignore_json() {
    let dir = scratch("completions-json");
    let plain = bin()
        .args(["completions", "bash"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let json = bin()
        .args(["--json", "completions", "bash"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(0), "{}", stderr(&json));
    assert_eq!(plain.stdout, json.stdout);
    assert!(!dir.join(".kaanal").exists());
}

#[test]
fn completions_unknown_shell_is_64() {
    let out = bin().args(["completions", "tcsh"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

#[test]
fn remote_down_with_a_tiny_budget_still_reports_stopped() {
    let up = Up::start("remote-down-tiny");
    let (port, token) = port_and_token(&up);
    let url = format!("http://127.0.0.1:{port}");
    let out = remote(
        &scratch("remote-down-tiny-elsewhere"),
        &["--endpoint", &url, "down", "--timeout", "1ms"],
        &[("KAANAL_TOKEN", &token)],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "stopped");
}

#[test]
fn completions_survive_a_closed_pipe() {
    let dir = scratch("completions-pipe");
    let direct = bin()
        .args(["completions", "bash"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(direct.status.code(), Some(0));
    assert!(stdout(&direct).contains("_kaanal"));
    for cmd in ["completions", "schema"] {
        let mut child = bin()
            .args([cmd, "bash"][..if cmd == "schema" { 1 } else { 2 }].iter())
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut first = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut first)
            .unwrap();
        // The reader is dropped here: the pipe is closed with output unread.
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{cmd}: {}", stderr(&out));
        assert_ne!(first, "");
    }
}

#[test]
fn a_url_after_the_subcommand_never_echoes_its_password() {
    let up = Up::start("endpoint-hint");
    for args in [
        &[
            "chaos",
            "set",
            "--endpoint",
            "http://u:hunter2@h:1/x",
            "--down",
        ][..],
        &["logs", "--endpoint", "http://u:hunter2@h:1/x"][..],
    ] {
        let out = up.run(args);
        assert_ne!(out.status.code(), Some(0), "{args:?}");
        let e = stderr(&out);
        assert!(!e.contains("hunter2"), "{args:?}: {e}");
    }
}
