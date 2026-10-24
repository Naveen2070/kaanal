//! `kaanal logs` and `trace`, including `--follow`.

use crate::common::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

impl Up {
    /// A GET of `path` on the mock, with an optional client `X-Request-Id`; returns the status
    /// and the `X-Request-Id` the server answered with.
    pub(crate) fn hit(&self, path: &str, rid: Option<&str>) -> (u16, String) {
        let run: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(self.dir.join(".kaanal/run.json")).unwrap(),
        )
        .unwrap();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .build()
            .into();
        let mut req = agent.get(&format!("http://127.0.0.1:{}{path}", run["port"]));
        if let Some(r) = rid {
            req = req.header("X-Request-Id", r);
        }
        let resp = req.call().unwrap();
        let id = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (resp.status().as_u16(), id)
    }
}

/// A `kaanal logs -f` child; its stdout lines arrive on a channel. Killed on drop.
pub(crate) struct Follow {
    pub(crate) child: Child,
    pub(crate) rx: std::sync::mpsc::Receiver<String>,
    /// Its stderr lines (notices).
    pub(crate) err: std::sync::mpsc::Receiver<String>,
}

impl Follow {
    pub(crate) fn start(up: &Up, args: &[&str]) -> Follow {
        let mut child = bin()
            .args(["logs", "-f"])
            .args(args)
            .current_dir(&up.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pump = |r: Box<dyn std::io::Read + Send>| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for l in BufReader::new(r).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
            rx
        };
        let rx = pump(Box::new(child.stdout.take().unwrap()));
        let err = pump(Box::new(child.stderr.take().unwrap()));
        Follow { child, rx, err }
    }

    /// The exit code once the child has ended, waiting up to `secs`.
    pub(crate) fn exit_code(&mut self, secs: u64) -> Option<i32> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s.code();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub(crate) fn line(&self) -> String {
        self.rx
            .recv_timeout(Duration::from_secs(15))
            .expect("follow printed no line in time")
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn logs_default_n_and_each_filter() {
    let up = Up::start("logs-filters");
    for _ in 0..3 {
        up.hit("/health", None);
    }
    up.hit("/nope", None);
    ok(
        &up,
        &[
            "chaos",
            "set",
            "--route",
            "GET /users/{id}",
            "--error-rate",
            "100%",
            "--error",
            "503",
        ],
    );
    assert_eq!(up.hit("/users/1", None).0, 503);
    let all = ok(&up, &["logs"]);
    assert_eq!(all.lines().count(), 5, "{all}");
    assert!(all.contains("GET /health  200"), "{all}");
    assert_eq!(ok(&up, &["logs", "-n", "2"]).lines().count(), 2);
    let r = ok(&up, &["logs", "--status", "404"]);
    assert!(
        r.lines().count() == 1 && r.contains("/nope") && r.contains("unmatched"),
        "{r}"
    );
    assert_eq!(ok(&up, &["logs", "--status", "404,5xx"]).lines().count(), 2);
    let r = ok(&up, &["logs", "--failed"]);
    assert!(
        r.lines().count() == 1 && r.contains("/users/1") && r.contains("failed="),
        "{r}"
    );
    let r = ok(&up, &["logs", "--route", "GET /users/{id}"]);
    assert!(r.lines().count() == 1 && r.contains("503"), "{r}");
    assert_eq!(ok(&up, &["logs", "--scope", "nobody"]), "");
    assert!(usage_fails(&up, &["logs", "--status", "9xx"]).contains("status"));
    assert!(usage_fails(&up, &["logs", "--scope", ""]).contains("scope"));
}

#[test]
fn logs_since_and_json_lines() {
    let up = Up::start("logs-json");
    up.hit("/health", Some("json-1"));
    up.hit("/health", None);
    let out = ok(&up, &["logs", "--json"]);
    let rows: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2, "{out}");
    assert_eq!(rows[0]["client_id"], "json-1");
    assert!(rows[0]["hops"].is_array());
    assert_eq!(ok(&up, &["logs", "--since", "1h"]).lines().count(), 2);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(ok(&up, &["logs", "--since", "10ms"]), "");
    assert!(usage_fails(&up, &["logs", "--since", "yesterday"]).contains("since"));
}

#[test]
fn trace_last_by_id_and_by_client_id() {
    let up = Up::start("trace-ok");
    let (_, id) = up.hit("/users/1", Some("my-req-9"));
    up.hit("/health", None);
    let last = ok(&up, &["trace", "last"]);
    assert!(last.contains("GET /health"), "{last}");
    for key in [id.as_str(), "my-req-9"] {
        let t = ok(&up, &["trace", key]);
        assert!(t.starts_with(&id) && t.contains("GET /users/1"), "{t}");
        assert!(
            t.contains("client=my-req-9") && t.contains('#') && t.contains('|'),
            "{t}"
        );
        assert!(t.contains("db.read"), "{t}");
    }
    let j = ok(&up, &["trace", &id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&j).unwrap();
    assert_eq!(v["id"], id.as_str());
}

#[test]
fn trace_evicted_unknown_and_exit_codes() {
    let up = Up::start("trace-errs");
    assert!(fails(&up, &["trace", "last"]).contains("no requests recorded yet"));
    let (_, id) = up.hit("/health", None);
    ok(&up, &["reset"]);
    let e = fails(&up, &["trace", &id]);
    assert!(
        e.contains("404") && e.contains("evicted") && e.contains("older than"),
        "{e}"
    );
    assert!(fails(&up, &["trace", "r0000-0001"]).contains("unknown"));
    assert!(fails(&up, &["trace", "no-such-client"]).contains("unknown"));
    assert!(fails(&up, &["trace", "bad id!"]).contains("400"));
    let dir = scratch("trace-down");
    let out = bin()
        .args(["trace", "last"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = bin()
        .args(["logs", "-f"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn logs_follow_prints_tail_then_new_requests_without_duplicates() {
    let up = Up::start("follow-order");
    for i in 0..3 {
        up.hit("/health", Some(&format!("old-{i}")));
    }
    ok(
        &up,
        &[
            "chaos",
            "set",
            "--route",
            "GET /users/{id}",
            "--latency",
            "800ms",
        ],
    );
    let f = Follow::start(&up, &["-n", "2"]);
    let tail = [f.line(), f.line()];
    assert!(
        tail[0].contains("old-1") && tail[1].contains("old-2"),
        "{tail:?}"
    );
    // The slow request starts first and finishes last.
    let slow = std::thread::scope(|s| {
        let slow = s.spawn(|| up.hit("/users/1", Some("slow-1")));
        std::thread::sleep(Duration::from_millis(200));
        up.hit("/health", Some("fast-1"));
        let first = f.line();
        assert!(first.contains("client=fast-1"), "{first}");
        slow.join().unwrap()
    });
    assert_eq!(slow.0, 200);
    let last = f.line();
    assert!(last.contains("client=slow-1"), "{last}");
    assert!(
        f.rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "unexpected extra line"
    );
}

#[test]
fn logs_follow_survives_a_request_finishing_between_tail_and_stream() {
    let up = Up::start("follow-race");
    let total = 60;
    std::thread::scope(|s| {
        let sender = s.spawn(|| {
            for i in 0..total {
                up.hit("/health", Some(&format!("race-{i}")));
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        let f = Follow::start(&up, &["-n", "1000"]);
        let mut seen = std::collections::HashSet::new();
        while seen.len() < total {
            let l = f.line();
            let cid = l.split("client=").nth(1).unwrap_or("").trim().to_string();
            assert!(seen.insert(cid.clone()), "duplicate line for {cid}: {l}");
        }
        sender.join().unwrap();
        assert!(
            f.rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "extra line"
        );
    });
}

#[test]
fn logs_rejects_n_outside_1_to_1000_and_a_closed_pipe_is_quiet() {
    let up = Up::start("logs-n");
    for n in ["0", "1001"] {
        let e = usage_fails(&up, &["logs", "-n", n]);
        assert!(e.contains("-n") && e.contains("1000"), "{e}");
    }
    ok(&up, &["logs", "-n", "1000"]);
    for _ in 0..30 {
        up.hit("/health", None);
    }
    // `kaanal logs | head` with the reader already gone must not panic or fail
    for args in [&["logs"][..], &["trace", "last"][..]] {
        let mut child = bin()
            .args(args)
            .current_dir(&up.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert_eq!(stderr(&out), "", "{args:?}");
    }
}

#[test]
fn logs_follow_filters_endpoint_failed_and_json_live() {
    let up = Up::start("follow-filters");
    up.hit("/users/1", Some("pre-1"));
    ok(
        &up,
        &[
            "chaos",
            "set",
            "--route",
            "GET /users/{id}",
            "--error-rate",
            "100%",
            "--error",
            "503",
        ],
    );
    up.hit("/users/2", Some("pre-f"));
    // The tail is printed after the subscription, so a tail line means the follower is live.
    let by_endpoint = Follow::start(&up, &["--route", "GET /users/{id}"]);
    assert!(by_endpoint.line().contains("client=pre-1"));
    assert!(by_endpoint.line().contains("client=pre-f"));
    let failed_json = Follow::start(&up, &["--failed", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&failed_json.line()).unwrap();
    assert_eq!(v["client_id"], "pre-f");

    up.hit("/health", Some("h-1"));
    up.hit("/users/3", Some("live-f"));
    ok(&up, &["chaos", "clear"]);
    up.hit("/users/4", Some("live-ok"));

    assert!(by_endpoint.line().contains("client=live-f"));
    assert!(by_endpoint.line().contains("client=live-ok"));
    assert!(
        by_endpoint
            .rx
            .recv_timeout(Duration::from_millis(500))
            .is_err()
    );
    // --json prints the whole entry (it has hops), not the event summary
    let v: serde_json::Value = serde_json::from_str(&failed_json.line()).unwrap();
    assert_eq!(v["client_id"], "live-f");
    assert!(
        v["hops"].is_array() && v["endpoint"] == "GET /users/{id}",
        "{v}"
    );
    assert!(
        failed_json
            .rx
            .recv_timeout(Duration::from_millis(500))
            .is_err()
    );
}

#[test]
fn logs_follow_exits_2_when_the_server_dies() {
    let mut up = Up::start("follow-dies");
    up.hit("/health", None);
    let mut f = Follow::start(&up, &["-n", "1"]);
    f.line();
    up.child.kill().unwrap();
    up.child.wait().unwrap();
    assert_eq!(f.exit_code(15), Some(2));
}

#[test]
fn logs_follow_exits_2_on_down() {
    let up = Up::start("follow-down");
    up.hit("/health", None);
    let mut f = Follow::start(&up, &["-n", "1"]);
    f.line();
    let down = up.run(&["down"]);
    assert_eq!(down.status.code(), Some(0), "{}", stderr(&down));
    assert_eq!(f.exit_code(20), Some(2));
    let errs: Vec<String> = f.err.try_iter().collect();
    assert!(
        errs.iter()
            .any(|l| l.contains("kaanal stopped (the server shut down)")),
        "{errs:?}"
    );
}

#[test]
fn logs_follow_json_keeps_stdout_pure_json_lines_and_sends_notices_to_stderr() {
    let up = Up::start("follow-json-pure");
    up.hit("/health", None);
    let f = Follow::start(&up, &["-n", "1", "--json"]);
    f.line(); // the tail: the follower is subscribed
    // A burst far beyond the 1000-entry ring: the follower fetches each entry one by one and
    // falls behind, so early entries are gone by the time it asks (a "could not fetch" notice),
    // and the server may report dropped events too.
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(up.dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    let url = format!("http://127.0.0.1:{}/health", run["port"]);
    std::thread::scope(|s| {
        for _ in 0..4 {
            // one agent per thread: its connection is reused, so the burst is fast and cheap
            s.spawn(|| {
                let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).build().into();
                for _ in 0..400 {
                    agent
                        .get(&url)
                        .call()
                        .unwrap()
                        .body_mut()
                        .read_to_vec()
                        .unwrap();
                }
            });
        }
    });
    let notice = f
        .err
        .recv_timeout(Duration::from_secs(60))
        .expect("no notice on stderr");
    assert!(notice.starts_with("-- "), "{notice}");
    // stdout so far: every line is a JSON object, never a notice
    let mut n = 0;
    while let Ok(l) = f.rx.try_recv() {
        let v: serde_json::Value =
            serde_json::from_str(&l).unwrap_or_else(|_| panic!("not JSON: {l}"));
        assert!(v["id"].is_string(), "{l}");
        n += 1;
    }
    assert!(n > 0);
}
