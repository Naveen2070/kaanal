use sha2::Digest as _;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kaanal"));
    c.env_remove("KAANAL_JSON");
    c.env_remove("KAANAL_ENDPOINT");
    c.env_remove("KAANAL_TOKEN");
    c
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-server/tests/fixtures")
}

fn fixture() -> PathBuf {
    fixtures().join("kaanal.yaml")
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A `kaanal up` child on an OS-chosen port, killed on drop.
struct Up {
    child: Child,
    dir: PathBuf,
}

impl Up {
    fn start(name: &str) -> Up {
        Up::start_with(name, &[])
    }

    fn start_with(name: &str, extra: &[&str]) -> Up {
        Up::launch(scratch(name), extra, &fixture())
    }

    /// Starts in an existing project dir with its own `kaanal.yaml`.
    fn start_in(dir: PathBuf) -> Up {
        Up::launch(dir, &[], Path::new("kaanal.yaml"))
    }

    fn launch(dir: PathBuf, extra: &[&str], cfg: &Path) -> Up {
        let child = bin()
            .args(["up", "--port", "0"])
            .args(extra)
            .arg("-c")
            .arg(cfg)
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let run = dir.join(".kaanal/run.json");
        let deadline = Instant::now() + Duration::from_secs(20);
        while std::fs::read_to_string(&run)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_none()
        {
            assert!(
                Instant::now() < deadline,
                "kaanal up did not write run.json"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Up { child, dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        bin().args(args).current_dir(&self.dir).output().unwrap()
    }

    /// The status a plain GET of `path` gets from the running mock.
    fn get_status(&self, path: &str) -> u16 {
        let run: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(self.dir.join(".kaanal/run.json")).unwrap(),
        )
        .unwrap();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("http://127.0.0.1:{}{path}", run["port"]);
        agent.get(&url).call().unwrap().status().as_u16()
    }
}

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn validate_ok() {
    let out = bin()
        .args(["validate", "-c"])
        .arg(fixture())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn validate_cycle_fails_with_message() {
    let dir = scratch("cycle");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n  b: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: a, to: b }\n  - { from: b, to: a }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cycle"), "{}", stderr(&out));
}

#[test]
fn validate_bad_yaml_shows_file_line_col() {
    let dir = scratch("yaml");
    let file = dir.join("kaanal.yaml");
    std::fs::write(&file, "version: 1\nport: nope\nnodes: {}\n").unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("kaanal.yaml:2:"), "{err}");
}

#[test]
fn validate_prints_unsupported_keyword_warnings() {
    let dir = scratch("warn");
    std::fs::copy(
        fixtures().join("openapi-warn.yaml"),
        dir.join("openapi-warn.yaml"),
    )
    .unwrap();
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi-warn.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("warning: ") && err.contains("'pattern'"),
        "{err}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok: "));
}

#[test]
fn status_without_running_instance_exits_2() {
    let dir = scratch("status");
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("not running"));
}

#[test]
fn stale_run_file_exits_2() {
    let dir = scratch("stale");
    // Grab a free port, then release it so nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
    std::fs::write(
        dir.join(".kaanal/run.json"),
        format!(r#"{{"port":{port},"token":"x","pid":1}}"#),
    )
    .unwrap();
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("not running"), "{}", stderr(&out));
}

#[test]
fn live_status_chaos_and_errors() {
    let up = Up::start("live");

    let out = up.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("db") && text.contains("postgres"), "{text}");

    let out = up.run(&[
        "chaos",
        "set",
        "db",
        "--latency",
        "300ms",
        "--error-rate",
        "10%",
        "--error",
        "503",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = up.run(&["status"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("+300ms, 10% errors (503)"), "{text}");

    // A failure kind shows by name, and an endpoint can be targeted.
    let out = up.run(&[
        "chaos",
        "set",
        "db",
        "--kind",
        "hang",
        "--error-rate",
        "100%",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = up.run(&["status"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("100% errors (hang)"), "{text}");
    let out = up.run(&["chaos", "clear", "db"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = up.run(&["chaos", "set", "--route", "get /users/{id}", "--down"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(up.get_status("/users/42"), 503, "the endpoint is down");
    let text = String::from_utf8_lossy(&up.run(&["status"]).stdout).into_owned();
    assert!(
        text.contains("endpoint GET /users/{id}:") && text.contains("DOWN"),
        "{text}"
    );
    let out = up.run(&["chaos", "clear", "--route", "GET /users/{id}"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(up.get_status("/users/42"), 200, "cleared");

    // Server-side errors print the server's text and exit 1.
    let out = up.run(&["chaos", "set", "redis", "--down"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("unknown node 'redis'"),
        "{}",
        stderr(&out)
    );

    // Control characters echoed by the server are neutralised.
    let out = up.run(&["chaos", "set", "\u{1b}[31mx", "--down"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(!err.contains('\u{1b}') && err.contains("?[31mx"), "{err:?}");

    let out = up.run(&["chaos", "clear"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = up.run(&["status"]);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("errors"));

    // A second `up` in the same directory refuses to start.
    let out = up.run(&["up", "--port", "0", "-c", fixture().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("already running"), "{}", stderr(&out));
}

/// A running instance in a project dir with one inline scenario, `db-down`.
fn scenario_up(name: &str) -> (Up, PathBuf) {
    let dir = project(
        name,
        "scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n",
    );
    (Up::start_in(dir.clone()), dir)
}

/// Runs `args`, asserts exit 0 and returns stdout.
fn ok(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

/// Runs `args`, asserts exit 64 (usage error) and returns stderr.
fn usage_fails(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(
        out.status.code(),
        Some(64),
        "{args:?}: stdout {} stderr {}",
        stdout(&out),
        stderr(&out)
    );
    stderr(&out)
}

/// Runs `args`, asserts exit 1 and returns stderr.
fn fails(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stdout(&out));
    stderr(&out)
}

/// Every client command, run with `--json`: stdout is empty or all JSON Lines (J2).
#[test]
fn every_client_command_honours_json() {
    let (up, _dir) = scenario_up("json-all");
    let cmds: &[&[&str]] = &[
        &["status"],
        &["pins"],
        &["scenario", "list"],
        &["scenario", "show", "db-down"],
        &["scenario", "activate", "db-down"],
        &["scenario", "active"],
        &["scenario", "deactivate", "db-down"],
        &["scenario", "set", "db-down"],
        &["scenario", "clear"],
        &["chaos", "set", "db", "--latency", "50ms"],
        &["chaos", "clear", "db"],
        &["node", "down", "db", "--for", "30s"],
        &["node", "up", "db"],
        &["cycle", "GET /users", "--state", "loading"],
        &["cycle", "GET /users", "--off"],
        &["logs", "-n", "5"],
        &["reset"],
    ];
    for c in cmds {
        let mut args = c.to_vec();
        args.push("--json");
        let out = ok(&up, &args);
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                serde_json::from_str::<serde_json::Value>(line).is_ok(),
                "{c:?}: not JSON: {line}"
            );
        }
    }
    // A 204 prints nothing, a body prints exactly one line.
    assert_eq!(ok(&up, &["chaos", "clear", "db", "--json"]), "");
    assert_eq!(
        ok(&up, &["scenario", "activate", "db-down", "--json"])
            .lines()
            .count(),
        1
    );
}

#[test]
fn live_scenarios_activate_deactivate_and_errors() {
    let (up, dir) = scenario_up("scn-basic");

    let list = ok(&up, &["scenario", "list"]);
    assert!(
        list.contains("db-down") && list.contains("inline"),
        "{list}"
    );
    assert!(!list.contains("(active)"), "{list}");

    ok(&up, &["scenario", "activate", "db-down"]);
    assert!(ok(&up, &["scenario", "list"]).contains("(active)"));
    assert_eq!(up.get_status("/users/42"), 503, "db is down");
    let st = ok(&up, &["status"]);
    assert!(
        st.contains("DOWN") && st.contains("active scenarios: db-down"),
        "{st}"
    );
    let show = ok(&up, &["scenario", "show", "db-down"]);
    assert!(
        show.contains("(inline)") && show.contains("\"down\":true"),
        "{show}"
    );

    // deactivate says what happened
    assert!(ok(&up, &["scenario", "deactivate", "db-down"]).contains("deactivated db-down"));
    assert_eq!(up.get_status("/users/42"), 200);
    assert!(ok(&up, &["scenario", "deactivate", "db-down"]).contains("is not active"));

    ok(&up, &["scenario", "activate", "db-down"]);
    ok(&up, &["scenario", "set"]);
    assert_eq!(up.get_status("/users/42"), 200);
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "");

    // readable server errors
    let e = fails(&up, &["scenario", "activate", "nope"]);
    assert!(
        e.contains("404") && e.contains("unknown scenario 'nope'"),
        "{e}"
    );
    for reserved in ["active", "save", "reset"] {
        let e = usage_fails(&up, &["scenario", "show", reserved]);
        assert!(e.contains("reserved"), "{reserved}: {e}");
    }
    // a name defined in kaanal.yaml cannot be redefined by load
    let same = dir.join("db-down.yaml");
    std::fs::write(&same, "chaos:\n  - { node: db, latency_ms: 1 }\n").unwrap();
    let e = fails(&up, &["scenario", "load", same.to_str().unwrap()]);
    assert!(e.contains("409"), "{e}");
}

#[test]
fn scenario_load_checks_the_file_here() {
    let (up, dir) = scenario_up("scn-load");
    let write = |n: &str, t: &str| {
        let f = dir.join(n);
        std::fs::write(&f, t).unwrap();
        f
    };

    let good = write("bug-123.yaml", "chaos:\n  - { node: db, latency_ms: 5 }\n");
    ok(&up, &["scenario", "load", good.to_str().unwrap()]);
    assert!(ok(&up, &["scenario", "active"]).contains("bug-123"));
    assert!(ok(&up, &["status"]).contains("+5ms"));

    let bad = write("typo.yaml", "chaos:\n  - { node: db, latncy_ms: 5 }\n");
    let e = fails(&up, &["scenario", "load", bad.to_str().unwrap()]);
    assert!(e.contains("typo.yaml:2:"), "{e}");

    let badname = write("Bad Name.yaml", "chaos:\n  - { node: db, down: true }\n");
    let e = usage_fails(&up, &["scenario", "load", badname.to_str().unwrap()]);
    assert!(e.contains("invalid scenario name"), "{e}");

    // reserved keys are refused here, like the server would, and nothing is defined
    let lim = write(
        "lim.yaml",
        "limits: {}\nchaos:\n  - { node: db, down: true }\n",
    );
    let e = fails(&up, &["scenario", "load", lim.to_str().unwrap()]);
    assert!(e.contains("not supported"), "{e}");
    let list = ok(&up, &["scenario", "list"]);
    assert!(!list.contains("lim "), "{list}");
}

/// Asserts `e` holds `file:LINE:COL` for `file`.
fn assert_located(e: &str, file: &str) {
    let ok = e.match_indices(&format!("{file}:")).any(|(i, _)| {
        let rest = &e[i + file.len() + 1..];
        let (line, tail) = rest.split_at(rest.find(':').unwrap_or(0));
        !line.is_empty()
            && line.chars().all(|c| c.is_ascii_digit())
            && tail[1..].starts_with(|c: char| c.is_ascii_digit())
    });
    assert!(ok, "no {file}:line:col in {e}");
}

#[test]
fn scenario_load_places_shape_errors_in_the_file_and_sends_nothing() {
    let (up, dir) = scenario_up("scn-shape");
    let cases = [
        (
            "rate.yaml",
            "chaos:\n  - { node: db, error_rate: 2 }\n",
            "error_rate",
        ),
        (
            "kind.yaml",
            "chaos:\n  - { node: db, error_rate: 0.5, error_kind: boom }\n",
            "error_kind",
        ),
        (
            "dup.yaml",
            "chaos:\n  - { node: db, latency_ms: 1 }\n  - { node: db, down: true }\n",
            "same key",
        ),
        ("empty.yaml", "chaos: []\n", "at least one of chaos or pins"),
        (
            "status.yaml",
            "chaos:\n  - { node: db, error_rate: 0.5, error_status: 404 }\n",
            "500-504",
        ),
        (
            "slow.yaml",
            "chaos:\n  - { node: db, latency_ms: 999999 }\n",
            "latency_ms",
        ),
        ("nothing.yaml", "chaos:\n  - { node: db }\n", "sets nothing"),
        (
            "target.yaml",
            "chaos:\n  - { down: true }\n",
            "node or endpoint",
        ),
        (
            "path.yaml",
            "chaos:\n  - { endpoint: users, down: true }\n",
            "METHOD /path",
        ),
    ];
    for (f, text, want) in cases {
        let file = dir.join(f);
        std::fs::write(&file, text).unwrap();
        let e = fails(&up, &["scenario", "load", file.to_str().unwrap()]);
        assert_located(&e, f);
        assert!(e.contains(want), "{f}: {e}");
    }
    // nothing was defined or activated
    let list = ok(&up, &["scenario", "list"]);
    for (f, ..) in cases {
        assert!(
            !list.contains(&format!("{} ", f.trim_end_matches(".yaml"))),
            "{list}"
        );
    }
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "");
}

#[test]
fn scenario_save_and_reset() {
    let (up, dir) = scenario_up("scn-save");
    let eff_file = dir.join("scenarios/eff.yaml");

    ok(&up, &["chaos", "set", "db", "--latency", "7ms"]);
    ok(&up, &["scenario", "save", "mine"]);
    let yaml = std::fs::read_to_string(dir.join("scenarios/mine.yaml")).unwrap();
    assert!(
        yaml.contains("\"latency_ms\":7") && yaml.contains("\"node\":\"db\""),
        "{yaml}"
    );
    assert!(
        !yaml.contains("error_rate") && !yaml.contains("down"),
        "only set fields: {yaml}"
    );
    let e = fails(&up, &["scenario", "save", "mine"]);
    assert!(e.contains("already exists"), "{e}");
    ok(&up, &["chaos", "set", "db", "--latency", "9ms"]);
    ok(&up, &["scenario", "save", "mine", "--force"]);
    let yaml = std::fs::read_to_string(dir.join("scenarios/mine.yaml")).unwrap();
    assert!(yaml.contains("\"latency_ms\":9"), "forced over: {yaml}");
    let e = usage_fails(&up, &["scenario", "save", "x", "--from", "mine"]);
    assert!(e.contains("scenario:NAME"), "{e}");

    // --clear-live moves the live chaos into the scenario
    ok(&up, &["scenario", "save", "moved", "--clear-live"]);
    assert!(!ok(&up, &["status"]).contains("+9ms"));
    ok(&up, &["scenario", "activate", "moved"]);
    assert!(ok(&up, &["status"]).contains("+9ms"));
    ok(
        &up,
        &["scenario", "save", "copy", "--from", "scenario:mine"],
    );
    let read = |n: &str| std::fs::read_to_string(dir.join(format!("scenarios/{n}.yaml"))).unwrap();
    assert_eq!(read("copy"), read("mine"), "a copy has the same content");
    assert!(read("copy").contains("\"latency_ms\":9"));

    // from effective, then a fresh load gives the same chaos
    ok(&up, &["scenario", "save", "eff", "--from", "effective"]);
    assert!(eff_file.exists());
    let rows = |st: String| -> String {
        st.lines()
            .filter(|l| !l.starts_with("active scenarios"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let before = rows(ok(&up, &["status"]));
    ok(&up, &["scenario", "clear"]);
    assert!(!ok(&up, &["status"]).contains("+9ms"));
    let again = dir.join("eff2.yaml");
    std::fs::copy(&eff_file, &again).unwrap();
    ok(&up, &["scenario", "load", again.to_str().unwrap()]);
    assert_eq!(rows(ok(&up, &["status"])), before);

    // reset: scenarios off, runtime ones gone, file ones stay
    ok(&up, &["scenario", "activate", "db-down"]);
    ok(&up, &["reset"]);
    let st = ok(&up, &["status"]);
    assert!(
        !st.contains("+9ms") && st.contains("active scenarios: -"),
        "{st}"
    );
    let list = ok(&up, &["scenario", "list"]);
    assert!(list.contains("mine") && list.contains("db-down"), "{list}");
    assert!(!list.contains("eff2"), "runtime scenario undefined: {list}");
}

#[test]
fn status_shows_each_failure_band_at_its_own_share() {
    let dir = project(
        "scn-bands",
        "scenarios:\n  a: { chaos: [ { node: db, error_rate: 0.3, error_kind: reset } ] }\n  b: { chaos: [ { node: db, error_rate: 0.3 } ] }\n",
    );
    let up = Up::start_in(dir);
    ok(&up, &["scenario", "set", "a", "b"]);
    // b is on top: 30% of requests fail with its 503, and of the rest 30% reset (21% overall).
    let st = ok(&up, &["status"]);
    assert!(
        st.contains("30% errors (503), 21% errors (reset)"),
        "not cumulative: {st}"
    );
    assert!(!st.contains("51%"), "{st}");
}

#[test]
fn status_shows_endpoint_rows_and_chaos_warns() {
    let (up, _dir) = scenario_up("scn-rows");
    let out = up.run(&["chaos", "set", "db", "--kind", "hang"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("warning:"), "{}", stderr(&out));
    ok(&up, &["chaos", "clear"]);

    let ep = "GET /users/{id}";
    ok(&up, &["chaos", "set", "--route", ep, "--latency", "9ms"]);
    ok(
        &up,
        &["chaos", "set", "db", "--route", ep, "--latency", "11ms"],
    );
    let st = ok(&up, &["status"]);
    assert!(st.contains("endpoint GET /users/{id}: +9ms"), "{st}");
    assert!(st.contains("db @ GET /users/{id}: +11ms"), "{st}");
}

#[test]
fn chaos_set_without_any_setting_is_an_error_not_an_empty_patch() {
    let dir = scratch("empty-patch");
    let out = bin()
        .args(["chaos", "set", "db"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("nothing to set"), "{}", stderr(&out));
}

fn spawn_up(dir: &Path) -> Child {
    bin()
        .args(["up", "--port", "0", "-c"])
        .arg(fixture())
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn run_json_pid(dir: &Path) -> Option<u64> {
    let s = std::fs::read_to_string(dir.join(".kaanal/run.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&s).ok()?["pid"].as_u64()
}

#[test]
fn killed_instance_does_not_block_the_next_up() {
    // The OS releases the lock when a process dies, so a crash never leaves a stuck "already running".
    let dir = scratch("killed");
    let mut first = spawn_up(&dir);
    let first_pid = u64::from(first.id());
    let deadline = Instant::now() + Duration::from_secs(20);
    while run_json_pid(&dir) != Some(first_pid) {
        assert!(Instant::now() < deadline, "first up never wrote run.json");
        std::thread::sleep(Duration::from_millis(50));
    }
    first.kill().unwrap();
    first.wait().unwrap();

    let mut second = spawn_up(&dir);
    let second_pid = u64::from(second.id());
    while run_json_pid(&dir) != Some(second_pid) {
        assert!(
            Instant::now() < deadline,
            "second up did not start after the first was killed"
        );
        if let Some(code) = second.try_wait().unwrap() {
            panic!("second up exited early with {code}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = bin().arg("status").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let _ = second.kill();
    let _ = second.wait();
}

#[test]
fn two_simultaneous_ups_let_exactly_one_win() {
    let dir = scratch("race");
    let mut a = spawn_up(&dir);
    let mut b = spawn_up(&dir);
    let deadline = Instant::now() + Duration::from_secs(20);
    // The loser exits quickly with code 1; the winner keeps running.
    let (loser_code, winner) = loop {
        if let Some(code) = a.try_wait().unwrap() {
            break (code, &mut b);
        }
        if let Some(code) = b.try_wait().unwrap() {
            break (code, &mut a);
        }
        assert!(Instant::now() < deadline, "neither instance exited");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(loser_code.code(), Some(1));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        winner.try_wait().unwrap().is_none(),
        "the winning instance must still be running"
    );
    let _ = winner.kill();
    let _ = winner.wait();
}

#[cfg(windows)]
#[test]
fn run_file_is_restricted_to_the_current_user_on_windows() {
    let up = Up::start("acl");
    let target = up.dir.join(".kaanal").join("run.json");
    let root = std::env::var("SystemRoot").unwrap();
    let out = Command::new(format!(r"{root}\System32\icacls.exe"))
        .arg(&target)
        .output()
        .unwrap();
    let acl = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let user = std::env::var("USERNAME").unwrap().to_lowercase();
    assert!(acl.contains(&user), "current user missing: {acl}");
    for other in ["everyone", "builtin\\users", "authenticated users"] {
        assert!(!acl.contains(other), "{other} must not have access: {acl}");
    }
}

#[test]
fn cli_reaches_an_instance_bound_to_ipv6_loopback() {
    let up = Up::start_with("ipv6", &["--host", "::1"]);
    let out = up.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn bad_arguments_exit_64_not_2() {
    // 2 means "not running" (R6.3); clap's default for usage errors is 2.
    for args in [&["chaos", "set", "db", "--error", "70000"][..], &["nope"]] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
    }
    let out = bin().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn spec_text_cannot_drive_the_terminal() {
    let dir = scratch("esc");
    std::fs::write(
        dir.join("openapi.yaml"),
        "openapi: 3.0.3\ninfo: { title: t, version: \"1\" }\npaths:\n  /x:\n    get:\n      responses:\n        \"200\":\n          description: ok\n          content:\n            application/json:\n              schema: { type: object, properties: { \"\\x1b[31mx\": { type: string, pattern: \"^a\" } } }\n",
    )
    .unwrap();
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    let err = stderr(&out);
    assert!(
        err.contains("?[31mx"),
        "warning should name the field: {err:?}"
    );
    assert!(!err.contains('\u{1b}'), "{err:?}");
}

#[test]
fn sample_project_validates() {
    // The shipped example differs from the test fixture (lognormal costs, port 8080), so check it too.
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nginx-server/kaanal.yaml");
    let out = bin()
        .args(["validate", "-c"])
        .arg(&sample)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "the sample should have no warnings: {}",
        stderr(&out)
    );
}

#[test]
fn validate_places_errors_in_later_nodes_and_edges() {
    let dir = scratch("positions");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1
nodes:
  app: { type: service }
  db: { type: database, flavor: postgres, bogus: 1 }
",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("kaanal.yaml:4:") && err.contains("node 'db': unknown field 'bogus'"),
        "{err}"
    );
    std::fs::write(
        &file,
        "version: 1
nodes:
  a: { type: service }
  b: { type: service }
edges:
  - { from: a, to: b }
  - { from: b, to: a }
",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert!(
        stderr(&out).contains("kaanal.yaml:7:5: cycle: a -> b -> a"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn schema_prints_the_committed_schema() {
    let out = bin().arg("schema").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-core/kaanal.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(printed, committed);
}

#[test]
fn gatewayless_example_validates() {
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/gatewayless/kaanal.yaml");
    let out = bin()
        .args(["validate", "-c"])
        .arg(&sample)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no warnings expected: {}",
        stderr(&out)
    );
}

#[test]
fn up_prints_the_realistic_seed() {
    let dir = scratch("realistic");
    std::fs::copy(fixtures().join("openapi.yaml"), dir.join("openapi.yaml")).unwrap();
    let src = std::fs::read_to_string(fixture())
        .unwrap()
        .replace("seed: 42", "seed: 42\ndeterminism: realistic");
    std::fs::write(dir.join("kaanal.yaml"), src).unwrap();
    let mut child = bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    let line = BufReader::new(out)
        .lines()
        .map(Result::unwrap)
        .find(|l| l.starts_with("Seed:"));
    let _ = child.kill();
    let _ = child.wait();
    let line = line.expect("up prints a Seed: line under determinism: realistic");
    let n: u64 = line["Seed:".len()..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        line,
        format!(
            "Seed:  {n} (determinism: realistic; replay its timings and failures with determinism: controlled and seed: {n}; generated data then follows that seed too)"
        )
    );
}

#[test]
fn validate_rejects_routes_with_a_pointer() {
    let dir = scratch("routes");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  app: { type: service }\nroutes:\n  \"GET /health\": { body: 1 }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.yaml:4:1: routes: was replaced by endpoints: (see docs)"),
        "{}",
        stderr(&out)
    );
}

/// A project dir: the fixture spec and a `kaanal.yaml` of gateway, app and db (8 lines),
/// then `tail` from line 9.
fn project(name: &str, tail: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::copy(fixtures().join("openapi.yaml"), dir.join("openapi.yaml")).unwrap();
    std::fs::write(
        dir.join("kaanal.yaml"),
        format!("version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml }}\n  db: {{ type: database, flavor: postgres }}\nedges:\n  - {{ from: gw, to: app }}\n  - {{ from: app, to: db }}\n{tail}"),
    )
    .unwrap();
    dir
}

fn kaanal(dir: &Path, args: &[&str]) -> Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// What `kaanal endpoints` adds when only `GET /users` has an entry (D22).
const SKELETON: &str = "# Added by kaanal endpoints\n\"GET /users/{id}\": {} # owner app, op read\n\"POST /orders\": {} # owner app, op write\n";

#[test]
fn endpoints_command_prints_and_appends_only() {
    let dir = project("eps-append", "endpoints: kaanal.endpoints.yaml\n");
    let file = dir.join("kaanal.endpoints.yaml");
    let old = "# my notes\r\n\"GET /users\": { override: { body: [] } } # keep me\r\n";
    std::fs::write(&file, old).unwrap();
    let out = kaanal(&dir, &["endpoints"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), SKELETON);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        old,
        "printing never writes"
    );
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let appended = format!("{old}{}", SKELETON.replace('\n', "\r\n"));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        appended,
        "old text untouched, CRLF kept"
    );
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert!(stdout(&out).contains("nothing to add"), "{}", stdout(&out));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), appended);
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no stale warnings: {}",
        stderr(&out)
    );
}

#[test]
fn endpoints_write_refuses_inline_and_existing_files() {
    // Inline map: print only; kaanal.yaml is never rewritten (R1.17).
    let dir = project("eps-inline", "endpoints:\n  \"GET /users\": {}\n");
    let before = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Indented for pasting below `endpoints:`.
    let indented: String = SKELETON.lines().map(|l| ["  ", l, "\n"].concat()).collect();
    assert_eq!(stdout(&out), indented);
    assert!(
        stderr(&out).contains("endpoints: is inline"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        before
    );
    // ...and pasting it as printed gives a config that validates with nothing stale.
    std::fs::write(dir.join("kaanal.yaml"), format!("{before}{}", stdout(&out))).unwrap();
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    std::fs::write(dir.join("kaanal.yaml"), &before).unwrap();
    // No `endpoints:` key: --write creates kaanal.endpoints.yaml and says how to wire it.
    let dir = project("eps-new", "");
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let created = std::fs::read_to_string(dir.join("kaanal.endpoints.yaml")).unwrap();
    assert_eq!(
        created,
        "# Added by kaanal endpoints\n\"GET /users\": {} # owner app, op scan\n\"GET /users/{id}\": {} # owner app, op read\n\"POST /orders\": {} # owner app, op write\n"
    );
    assert!(
        stdout(&out).contains("endpoints: kaanal.endpoints.yaml"),
        "{}",
        stdout(&out)
    );
    // ...and refuses to touch that file while kaanal.yaml still does not name it.
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.endpoints.yaml exists"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.endpoints.yaml")).unwrap(),
        created
    );
}

#[test]
fn validate_warns_about_stale_entries_and_ignored_error_maps() {
    let dir = project(
        "eps-warn",
        "endpoints:\n  \"GET /old\": {}\n  \"GET /health\": { override: { body: 1 }, chain: [ { call: app, via: \"POST /orders\" } ] }\n  \"POST /orders\": { error_map: { \"5xx\": 502 } }\n",
    );
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("warning: endpoint 'GET /old': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)"), "{err}");
    assert!(err.contains("warning: endpoint 'GET /health': via 'POST /orders': that endpoint's error_map is ignored here (each endpoint maps its own errors)"), "{err}");
}

#[test]
fn validate_places_errors_in_the_endpoints_file() {
    let dir = project("eps-errors", "endpoints: kaanal.endpoints.yaml\n");
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.yaml:9:12: "),
        "a missing file, at the endpoints: value: {}",
        stderr(&out)
    );
    let file = dir.join("kaanal.endpoints.yaml");
    for (text, want) in [
        (
            "\"GET /users/{id}\":\n  chain:\n    - call: pay\n",
            "kaanal.endpoints.yaml:3:13: endpoint 'GET /users/{id}': unknown node 'pay'",
        ),
        (
            "\"GET /users/{id}\":\n  chain:\n    - { cache: c, op: read }\n",
            "kaanal.endpoints.yaml:3:7: ",
        ),
    ] {
        std::fs::write(&file, text).unwrap();
        let out = kaanal(&dir, &["validate"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains(want), "{}", stderr(&out));
    }
}

/// One PKCS#8 key per test binary, pre-seeded where a test needs a key (spec §7: only
/// `up_generates_the_key_once_then_reuses_it` lets kaanal generate one).
fn test_pem() -> &'static str {
    static PEM: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PEM.get_or_init(|| {
        kaanal_server::AuthKey::generate()
            .unwrap()
            .to_pem()
            .unwrap()
    })
}

/// Where kaanal keeps the key of the project in `project` under `cfg` (X14; the same formula
/// as `keys.rs`, so a change to it fails here too).
fn key_path(cfg: &Path, project: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(project).unwrap();
    let text = canon.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let id = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
    cfg.join("kaanal")
        .join("keys")
        .join(format!("{}.pem", &id[..16]))
}

/// An auth-only project (one user) and its private `KAANAL_CONFIG_DIR`; `seed` writes the
/// shared test key where kaanal looks for it.
fn auth_project(name: &str, seed: bool) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  login: { type: auth, proc: 1ms, users: [ { username: alice, password: s3cr3t-pw } ] }\n",
    )
    .unwrap();
    let cfg = dir.join("config-home");
    if seed {
        let key = key_path(&cfg, &dir);
        std::fs::create_dir_all(key.parent().unwrap()).unwrap();
        std::fs::write(&key, test_pem()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    (dir, cfg)
}

/// `kaanal up` in `project` with `KAANAL_CONFIG_DIR=cfg`: the child and its stdout lines up
/// to the `Auth:` line (or all of them, if it exits first).
fn up_auth(project: &Path, cfg: &Path) -> (Child, Vec<String>) {
    let mut child = bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(project)
        .env("KAANAL_CONFIG_DIR", cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = Vec::new();
    for l in BufReader::new(child.stdout.take().unwrap()).lines() {
        let l = l.unwrap();
        let last = l.starts_with("Auth:");
        lines.push(l);
        if last {
            break;
        }
    }
    (child, lines)
}

/// `kaanal up` in `project` that is expected to exit (an error).
fn up_fails(project: &Path, cfg: &Path) -> Output {
    bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(project)
        .env("KAANAL_CONFIG_DIR", cfg)
        .output()
        .unwrap()
}

fn port_of(lines: &[String]) -> u16 {
    lines
        .iter()
        .find_map(|l| l.strip_prefix("Mocks: http://localhost:"))
        .unwrap()
        .parse()
        .unwrap()
}

/// The `kid` the running instance serves in its JWKS.
fn served_kid(port: u16) -> serde_json::Value {
    let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).build().into();
    let jwks: serde_json::Value = agent
        .get(&format!("http://127.0.0.1:{port}/.well-known/jwks.json"))
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    jwks["keys"][0]["kid"].clone()
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn up_generates_the_key_once_then_reuses_it() {
    // The one key generation by kaanal itself in this test binary (spec §7).
    let (dir, cfg) = auth_project("keygen", false);
    let (child, lines) = up_auth(&dir, &cfg);
    let port = port_of(&lines);
    let kid = served_kid(port);
    stop(child);
    assert!(
        lines.iter().any(|l| l == "Generating signing key..."),
        "{lines:?}"
    );
    assert_eq!(
        lines.last().unwrap(),
        &format!(
            "Auth:  POST http://localhost:{port}/login, JWKS http://localhost:{port}/.well-known/jwks.json, 1 user"
        )
    );
    let keys = cfg.join("kaanal").join("keys");
    let files: Vec<String> = std::fs::read_dir(&keys)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let want = key_path(&cfg, &dir);
    assert_eq!(
        files,
        [want.file_name().unwrap().to_string_lossy().into_owned()],
        "one key file, no temp file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&keys), mode(&want)), (0o700, 0o600));
    }
    // The next run reuses it.
    let (child, lines) = up_auth(&dir, &cfg);
    let again = served_kid(port_of(&lines));
    stop(child);
    assert!(
        !lines.iter().any(|l| l.starts_with("Generating")),
        "{lines:?}"
    );
    assert_eq!(again, kid);
}

#[test]
fn up_uses_a_pre_seeded_key() {
    let (dir, cfg) = auth_project("seeded", true);
    let (child, lines) = up_auth(&dir, &cfg);
    let kid = served_kid(port_of(&lines));
    stop(child);
    assert!(
        !lines.iter().any(|l| l.starts_with("Generating")),
        "{lines:?}"
    );
    let want = kaanal_server::AuthKey::from_pem(test_pem()).unwrap();
    assert_eq!(kid, want.kid());
}

#[test]
fn corrupt_key_file_stops_startup_naming_it() {
    let (dir, cfg) = auth_project("corrupt-key", false);
    let key = key_path(&cfg, &dir);
    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
    std::fs::write(&key, "not a key").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let out = up_fails(&dir, &cfg);
    assert_eq!(out.status.code(), Some(1));
    let e = stderr(&out);
    let name = key.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        e.contains(&name) && e.contains("delete the file to generate a new key"),
        "{e}"
    );
    assert_eq!(
        std::fs::read_to_string(&key).unwrap(),
        "not a key",
        "never replaced"
    );
}

#[test]
fn two_simultaneous_ups_share_one_key() {
    // Different working directories, so both pass the per-directory instance lock; the same
    // config file, so both want the same key. Whoever links second adopts the first's key.
    let (dir, cfg) = auth_project("key-race", false);
    let config = dir.join("kaanal.yaml");
    let other = scratch("key-race-cwd");
    let start = |cwd: &Path| {
        bin()
            .args(["up", "--port", "0", "-c"])
            .arg(&config)
            .current_dir(cwd)
            .env("KAANAL_CONFIG_DIR", &cfg)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let read = |mut child: Child| {
        let mut lines = Vec::new();
        for l in BufReader::new(child.stdout.take().unwrap()).lines() {
            let l = l.unwrap();
            let last = l.starts_with("Auth:");
            lines.push(l);
            if last {
                break;
            }
        }
        (child, lines)
    };
    let (a, b) = (start(&dir), start(&other));
    let ((a, la), (b, lb)) = (read(a), read(b));
    let (ka, kb) = (served_kid(port_of(&la)), served_kid(port_of(&lb)));
    stop(a);
    stop(b);
    assert_eq!(ka, kb, "one key for one project: {la:?} {lb:?}");
    let names: Vec<String> = std::fs::read_dir(cfg.join("kaanal").join("keys"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "one key file and no temp file: {names:?}");
    assert_eq!(
        Path::new(&names[0]).extension().and_then(|e| e.to_str()),
        Some("pem"),
        "{names:?}"
    );
}

#[test]
fn up_with_a_broken_config_generates_no_key() {
    let (dir, cfg) = auth_project("broken-no-key", false);
    // A cache without a flavor is a topology error; `validate` reports it too.
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  login: { type: auth, users: [ { username: a, password: b } ] }\n  sess: { type: cache }\nedges:\n  - { from: login, to: sess }\n",
    )
    .unwrap();
    let out = up_fails(&dir, &cfg);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("cache node 'sess' needs"),
        "{}",
        stderr(&out)
    );
    assert!(!stdout(&out).contains("Generating"), "{}", stdout(&out));
    assert!(
        !cfg.exists(),
        "no key directory for a config that cannot run"
    );
}

#[test]
fn validate_never_touches_keys_and_warns_without_users() {
    let dir = scratch("auth-validate");
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  login: { type: auth }\n",
    )
    .unwrap();
    let cfg = dir.join("config-home");
    let out = bin()
        .arg("validate")
        .current_dir(&dir)
        .env("KAANAL_CONFIG_DIR", &cfg)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("warning: node 'login': no users, so every POST /login answers 401"),
        "{}",
        stderr(&out)
    );
    assert!(!cfg.exists(), "validate never creates the key directory");
}

#[test]
fn endpoints_command_skips_built_in_routes() {
    let dir = project("eps-auth", "");
    let yaml = std::fs::read_to_string(dir.join("kaanal.yaml"))
        .unwrap()
        .replace(
            "nodes:\n",
            "nodes:\n  login: { type: auth, users: [ { username: a, password: b } ] }\n",
        )
        + "  - { from: gw, to: login }\n";
    std::fs::write(dir.join("kaanal.yaml"), yaml).unwrap();
    let out = kaanal(&dir, &["endpoints"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "# Added by kaanal endpoints\n\"GET /users\": {} # owner app, op scan\n\"GET /users/{id}\": {} # owner app, op read\n\"POST /orders\": {} # owner app, op write\n"
    );
}

#[cfg(unix)]
#[test]
fn unix_key_files_readable_by_others_are_refused() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, cfg) = auth_project("wide-key", true);
    let key = key_path(&cfg, &dir);
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = up_fails(&dir, &cfg);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("chmod 600"), "{}", stderr(&out));
    // P16: the keys directory is made private on every run, even one that existed wider.
    let mode = std::fs::metadata(key.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[cfg(windows)]
#[test]
fn explicit_wide_grants_in_the_keys_dir_are_removed_on_windows() {
    let (dir, cfg) = auth_project("keys-wide-acl", true);
    let keys = cfg.join("kaanal").join("keys");
    let key = key_path(&cfg, &dir);
    let root = std::env::var("SystemRoot").unwrap();
    let icacls = format!(r"{root}\System32\icacls.exe");
    // A keys directory (and key) someone had opened to Everyone before kaanal ran.
    let out = Command::new(&icacls)
        .arg(&keys)
        .args(["/grant", "*S-1-1-0:(OI)(CI)R", "/T", "/C", "/Q"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let (child, _) = up_auth(&dir, &cfg);
    stop(child);
    for path in [&keys, &key] {
        let out = Command::new(&icacls).arg(path).output().unwrap();
        let acl = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(!acl.contains("everyone"), "{}: {acl}", path.display());
    }
}

#[cfg(windows)]
#[test]
fn keys_dir_is_restricted_to_the_current_user_on_windows() {
    let (dir, cfg) = auth_project("keys-acl", true);
    let (child, _) = up_auth(&dir, &cfg);
    stop(child);
    let keys = cfg.join("kaanal").join("keys");
    let root = std::env::var("SystemRoot").unwrap();
    let out = Command::new(format!(r"{root}\System32\icacls.exe"))
        .arg(&keys)
        .output()
        .unwrap();
    let acl = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let user = std::env::var("USERNAME").unwrap().to_lowercase();
    assert!(acl.contains(&user), "current user missing: {acl}");
    for other in ["everyone", "builtin\\users", "authenticated users"] {
        assert!(!acl.contains(other), "{other} must not have access: {acl}");
    }
}

#[test]
fn activate_with_ttl_lists_and_expires() {
    let (up, _dir) = scenario_up("scn-ttl");
    let out = ok(&up, &["scenario", "activate", "db-down", "--ttl", "300ms"]);
    assert!(out.contains("active scenarios: db-down"), "{out}");
    let line = |up: &Up| {
        ok(up, &["scenario", "list"])
            .lines()
            .find(|l| l.starts_with("db-down"))
            .unwrap()
            .to_string()
    };
    // The server may already have expired it on a slow box, so only check the format if live.
    let first = line(&up);
    assert!(
        first.contains("(active, expires in ") || !first.contains("(active"),
        "{first}"
    );
    let end = Instant::now() + Duration::from_secs(3);
    while line(&up).contains("(active") {
        assert!(Instant::now() < end, "never expired");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn bad_ttl_is_rejected_here() {
    let (up, _dir) = scenario_up("scn-badttl");
    let e = usage_fails(&up, &["scenario", "activate", "db-down", "--ttl", "5"]);
    assert!(e.contains("bad ttl"), "{e}");
    assert!(!ok(&up, &["scenario", "list"]).contains("(active"));
}

#[test]
fn active_stays_names_only() {
    let (up, _dir) = scenario_up("scn-ttlnames");
    ok(&up, &["scenario", "activate", "db-down", "--ttl", "30s"]);
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "db-down");
    let list = ok(&up, &["scenario", "list"]);
    assert!(list.contains("(active, expires in "), "{list}");
}

#[test]
fn cycle_steps_and_prints_the_state() {
    let (up, _dir) = scenario_up("cyc-steps");
    for want in ["loading", "empty", "error", "slow", "success", "loading"] {
        let out = ok(&up, &["cycle", "GET /users"]);
        assert!(
            out.contains(&format!("GET /users -> {want}")),
            "{want}: {out}"
        );
    }
}

#[test]
fn cycle_state_flag_and_off() {
    let (up, _dir) = scenario_up("cyc-flags");
    let out = ok(
        &up,
        &[
            "cycle",
            "GET /users",
            "--state",
            "error",
            "--status",
            "404",
            "--delay",
            "2s",
        ],
    );
    assert_eq!(
        out,
        "GET /users -> error (status 404, delay 2s)
"
    );
    assert!(ok(&up, &["pins"]).contains("GET /users"));
    let out = ok(&up, &["cycle", "GET /users", "--off"]);
    assert_eq!(
        out,
        "GET /users -> unpinned
"
    );
    assert_eq!(
        ok(&up, &["pins"]),
        "no pins
",
        "pin removed"
    );
    // no pin to remove: the server answers 204 either way
    let out = ok(&up, &["cycle", "GET /users", "--off"]);
    assert_eq!(
        out,
        "GET /users -> unpinned
"
    );
    // a braced key survives the query encoding
    ok(&up, &["cycle", "GET /users/{id}", "--state", "slow"]);
    assert!(ok(&up, &["pins"]).contains("GET /users/{id}"));
    ok(&up, &["cycle", "GET /users/{id}", "--off"]);
    assert_eq!(
        ok(&up, &["pins"]),
        "no pins
",
        "braced pin removed"
    );
    let e = up.run(&["cycle", "GET /users", "--off", "--state", "error"]);
    assert_ne!(e.status.code(), Some(0), "--off conflicts with --state");
}

#[test]
fn cycle_bad_state_rejected_here() {
    let (up, _dir) = scenario_up("cyc-bad");
    let e = usage_fails(&up, &["cycle", "GET /users", "--state", "broken"]);
    assert!(e.contains("loading") && e.contains("success"), "{e}");
    let e = usage_fails(
        &up,
        &["cycle", "GET /users", "--state", "slow", "--delay", "soon"],
    );
    assert!(e.contains("delay"), "{e}");
    let pins = ok(&up, &["pins"]);
    assert!(!pins.contains("GET /users"), "nothing was sent: {pins}");
}

#[test]
fn cycle_server_400_is_shown_and_changes_nothing() {
    let (up, _dir) = scenario_up("cyc-400");
    // the first step is loading, which takes no status: the server says so and pins nothing
    let e = fails(&up, &["cycle", "GET /users", "--status", "404"]);
    assert!(e.contains("HTTP 400"), "{e}");
    assert!(!ok(&up, &["pins"]).contains("GET /users"));
}

#[test]
fn pins_lists_live_and_effective_with_source() {
    let (up, dir) = scenario_up("pins-list");
    assert!(ok(&up, &["pins"]).contains("no pins"));
    let f = dir.join("pinned.yaml");
    std::fs::write(
        &f,
        "pins:\n  - { endpoint: \"GET /users/{id}\", state: error, status: 404 }\n",
    )
    .unwrap();
    ok(&up, &["scenario", "load", f.to_str().unwrap()]);
    ok(&up, &["cycle", "GET /users", "--state", "slow"]);
    let out = ok(&up, &["pins"]);
    let want = format!(
        "live pins:
  {:<24} slow
effective pins:
  {:<24} {:<8} live
  {:<24} {:<8} scenario:pinned (status 404)
",
        "GET /users", "GET /users", "slow", "GET /users/{id}", "error"
    );
    assert_eq!(out, want);
    // a scenario-only pin leaves the live section empty
    ok(&up, &["cycle", "GET /users", "--off"]);
    let out = ok(&up, &["pins"]);
    let want = format!(
        "no live pins
effective pins:
  {:<24} {:<8} scenario:pinned (status 404)
",
        "GET /users/{id}", "error"
    );
    assert_eq!(out, want);
}

#[test]
fn status_shows_pins() {
    let (up, _dir) = scenario_up("pins-status");
    assert!(!ok(&up, &["status"]).contains("pin "));
    ok(&up, &["cycle", "GET /users", "--state", "error"]);
    let st = ok(&up, &["status"]);
    assert!(
        st.lines().any(|l| l == "pin GET /users: error (live)"),
        "{st}"
    );
}

#[test]
fn scenario_show_prints_pins() {
    let (up, dir) = scenario_up("pins-show");
    let f = dir.join("shown.yaml");
    std::fs::write(
        &f,
        "pins:\n  - { endpoint: \"GET /users\", state: error, status: 404, delay: 2s }\n",
    )
    .unwrap();
    ok(&up, &["scenario", "load", f.to_str().unwrap()]);
    let show = ok(&up, &["scenario", "show", "shown"]);
    assert!(
        show.contains("pin GET /users: error")
            && show.contains("status 404")
            && show.contains("delay 2s"),
        "{show}"
    );
}

#[test]
fn scenario_load_accepts_pins_only_file_and_still_refuses_limits() {
    let (up, dir) = scenario_up("pins-load");
    let write = |n: &str, t: &str| {
        let f = dir.join(n);
        std::fs::write(&f, t).unwrap();
        f
    };
    let good = write(
        "ui-err.yaml",
        "pins:\n  - { endpoint: \"GET /users\", state: loading }\n",
    );
    ok(&up, &["scenario", "load", good.to_str().unwrap()]);
    assert!(ok(&up, &["scenario", "active"]).contains("ui-err"));

    let bad = write(
        "bad-pin.yaml",
        "pins:\n  - { endpoint: \"GET /users\", state: slow, status: 200 }\n",
    );
    let e = fails(&up, &["scenario", "load", bad.to_str().unwrap()]);
    assert_located(&e, "bad-pin.yaml");
    assert!(!ok(&up, &["scenario", "list"]).contains("bad-pin "));

    let lim = write(
        "lim2.yaml",
        "limits: {}\npins:\n  - { endpoint: \"GET /users\", state: loading }\n",
    );
    let e = fails(&up, &["scenario", "load", lim.to_str().unwrap()]);
    assert!(e.contains("not supported"), "{e}");
}

// ---- logs and trace (C1) ----

impl Up {
    /// A GET of `path` on the mock, with an optional client `X-Request-Id`; returns the status
    /// and the `X-Request-Id` the server answered with.
    fn hit(&self, path: &str, rid: Option<&str>) -> (u16, String) {
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
struct Follow {
    child: Child,
    rx: std::sync::mpsc::Receiver<String>,
    /// Its stderr lines (notices).
    err: std::sync::mpsc::Receiver<String>,
}

impl Follow {
    fn start(up: &Up, args: &[&str]) -> Follow {
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
    fn exit_code(&mut self, secs: u64) -> Option<i32> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s.code();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn line(&self) -> String {
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

/// Runs the CLI in a fresh scratch dir (no instance running) and returns the exit code and stderr.
static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
use std::sync::atomic::Ordering;

fn exit_of(args: &[&str]) -> (Option<i32>, String) {
    let out = bin()
        .args(args)
        .current_dir(scratch(&format!(
            "usage-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
        .output()
        .unwrap();
    (out.status.code(), stderr(&out))
}

fn assert_64(args: &[&str]) {
    let (code, err) = exit_of(args);
    assert_eq!(code, Some(64), "{args:?}: {err}");
}

#[test]
fn bad_log_filter_exits_64_without_an_instance_in_both_modes() {
    for args in [
        &["logs", "--status", "9xx"][..],
        &["logs", "-f", "--status", "9xx"],
        &["logs", "--since", "yesterday"],
        &["logs", "-f", "--since", "yesterday"],
        &["logs", "--scope", ""],
        &["logs", "-f", "--scope", ""],
    ] {
        assert_64(args);
    }
}

#[test]
fn bad_names_in_set_and_save_exit_64() {
    assert_64(&["scenario", "set", "ok", "Bad Name"]);
    assert_64(&["scenario", "save", "Bad Name"]);
}

#[test]
fn unknown_flag_exits_64() {
    assert_64(&["status", "--bogus"]);
}

#[test]
fn unknown_subcommand_exits_64() {
    assert_64(&["frobnicate"]);
}

#[test]
fn missing_required_arg_exits_64() {
    assert_64(&["trace"]);
    assert_64(&["chaos", "set"]);
}

#[test]
fn conflicting_flags_exit_64() {
    assert_64(&["cycle", "GET /x", "--off", "--state", "error"]);
}

#[test]
fn help_and_version_exit_0() {
    for args in [
        &["--help"][..],
        &["-V"],
        &["--version"],
        &["help", "chaos"],
        &["chaos", "set", "--help"],
    ] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn bare_command_without_subcommand_exits_64() {
    assert_64(&["chaos"]);
    assert_64(&[]);
}

#[test]
fn bad_ttl_exits_64() {
    assert_64(&["scenario", "activate", "db-down", "--ttl", "5"]);
}

#[test]
fn bad_scenario_name_exits_64() {
    assert_64(&["scenario", "show", "active"]);
    assert_64(&["scenario", "deactivate", "Bad Name"]);
}

#[test]
fn bad_state_exits_64() {
    assert_64(&["cycle", "GET /x", "--state", "broken"]);
}

#[test]
fn bad_delay_exits_64() {
    assert_64(&["cycle", "GET /x", "--delay", "soon"]);
}

#[test]
fn bad_from_exits_64() {
    assert_64(&["scenario", "save", "x", "--from", "mine"]);
}

#[test]
fn n_out_of_range_exits_64() {
    assert_64(&["logs", "-n", "0"]);
    assert_64(&["logs", "-n", "1001"]);
}

#[test]
fn empty_scope_exits_64() {
    assert_64(&["logs", "--scope", ""]);
}

#[test]
fn chaos_set_nothing_exits_64() {
    assert_64(&["chaos", "set", "db"]);
}

#[test]
fn bad_latency_exits_64() {
    assert_64(&["chaos", "set", "db", "--latency", "15"]);
}

#[test]
fn bad_rate_exits_64() {
    assert_64(&["chaos", "set", "db", "--error-rate", "150%"]);
}

#[test]
fn scenario_load_bad_file_name_exits_64() {
    let d = scratch("usage-load");
    let f = d.join("Bad Name.yaml");
    std::fs::write(
        &f,
        "chaos: []
",
    )
    .unwrap();
    assert_64(&["scenario", "load", f.to_str().unwrap()]);
}

#[test]
fn a_server_400_is_still_exit_1() {
    let up = Up::start("usage-400");
    // parses here, the server refuses the id
    let e = fails(&up, &["trace", "bad id!"]);
    assert!(e.contains("HTTP 400"), "{e}");
}

// ---- lifecycle: stale state, startup window, stop paths (C2a task 3) ----

fn run_json(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap()).unwrap()
}

/// Waits (bounded) for `dir`'s run.json to carry `pid`, failing early if the child exits.
fn wait_for_pid(dir: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while run_json_pid(dir) != Some(u64::from(child.id())) {
        if let Some(code) = child.try_wait().unwrap() {
            panic!("up exited early with {code}");
        }
        assert!(Instant::now() < deadline, "up never wrote its run.json");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits (bounded) for the child to exit and returns its code.
fn wait_exit(child: &mut Child, secs: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s.code();
        }
        assert!(Instant::now() < deadline, "up did not exit in {secs}s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

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
    assert!(ui.ends_with("/__kaanal/ui"), "{ui}");
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
    assert!(ui.unwrap().contains("/__kaanal/ui#token="));
}

/// A lock held by this test process, standing in for a running (or still starting) `up`.
fn hold_lock(dir: &Path) -> std::fs::File {
    std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
    let f = std::fs::File::create(dir.join(".kaanal/lock")).unwrap();
    f.try_lock().unwrap();
    f
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
struct StopListener(
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

/// Kills (and reaps) a helper process on drop, so a failing assert never leaks it.
struct Helper(Child);

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

fn down_in(dir: &Path, args: &[&str]) -> Output {
    bin()
        .arg("down")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A TCP port nothing listens on.
fn closed_port() -> u16 {
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

/// Stops whatever daemon lives in `dir` when dropped, even after a failed assertion: `down`, then
/// a hard kill of the pid in run.json if one is left.
struct Daemon(PathBuf);

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
fn up_d_cmd(dir: &Path, cfg: &Path) -> Command {
    let mut c = bin();
    c.args(["up", "-d", "--port", "0", "-c"])
        .arg(cfg)
        .current_dir(dir);
    c
}

/// Reads both pipes of `child` to EOF on threads; the receiver gets `(0|1, bytes)` per pipe.
fn drain_pipes(child: &mut Child) -> std::sync::mpsc::Receiver<(usize, Vec<u8>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let pipes: [Box<dyn Read + Send>; 2] = [
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    for (i, mut pipe) in pipes.into_iter().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = pipe.read_to_end(&mut b);
            let _ = tx.send((i, b));
        });
    }
    rx
}

/// Runs `cmd` with both pipes read to EOF, failing if EOF does not come within 30 s: a daemon that
/// inherited the pipes would hold them open forever.
fn piped(mut cmd: Command) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let rx = drain_pipes(&mut child);
    let mut bufs = [Vec::new(), Vec::new()];
    for _ in 0..2 {
        let (i, b) = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("pipes not closed in 30 s: the daemon holds the caller's stdout/stderr");
        bufs[i] = b;
    }
    let [stdout, stderr] = bufs;
    Output {
        status: child.wait().unwrap(),
        stdout,
        stderr,
    }
}

fn mocks_port(out: &Output) -> u16 {
    stdout(out)
        .lines()
        .find_map(|l| l.strip_prefix("Mocks: http://localhost:"))
        .unwrap_or_else(|| panic!("no Mocks line: {} / {}", stdout(out), stderr(out)))
        .parse()
        .unwrap()
}

fn health(port: u16) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    agent
        .get(&format!("http://127.0.0.1:{port}/__kaanal/health"))
        .header("Host", &format!("localhost:{port}"))
        .call()
        .unwrap()
        .status()
        .as_u16()
}

fn run_token(dir: &Path) -> String {
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
fn kaanal_server_lock_free(dir: &Path) -> bool {
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

/// Parses stdout that must be exactly one JSON value on one line.
fn one_json(out: &str) -> serde_json::Value {
    assert_eq!(out.trim_end().lines().count(), 1, "{out}");
    serde_json::from_str(out).unwrap_or_else(|e| panic!("not JSON ({e}): {out}"))
}

#[test]
fn json_flag_works_before_and_after_the_subcommand() {
    let up = Up::start("json-pos");
    for args in [["status", "--json"], ["--json", "status"]] {
        let rows = one_json(&ok(&up, &args));
        let rows = rows.as_array().expect("an array");
        assert!(
            rows.iter()
                .all(|r| r["node"].is_string() && r["type"].is_string())
        );
        assert!(rows.iter().all(|r| r["state"] == "up"), "{rows:?}");
        assert!(rows.iter().any(|r| r["node"] == "db"));
    }
}

#[test]
fn json_env_switches_it_on_and_empty_env_is_off() {
    let up = Up::start("json-env");
    let run = |v: &str| {
        let out = bin()
            .arg("status")
            .env("KAANAL_JSON", v)
            .current_dir(&up.dir)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        stdout(&out)
    };
    assert!(one_json(&run("1")).is_array());
    assert!(one_json(&run("true")).is_array());
    assert!(run("").starts_with("NODE"));
}

#[test]
fn json_pins_and_scenario_reads_parse() {
    let (up, _dir) = scenario_up("json-reads");
    assert!(one_json(&ok(&up, &["pins", "--json"])).is_object());
    assert!(one_json(&ok(&up, &["scenario", "list", "--json"])).is_array());
    assert!(one_json(&ok(&up, &["scenario", "active", "--json"])).is_object());
    let v = one_json(&ok(&up, &["scenario", "show", "db-down", "--json"]));
    assert!(v["chaos"].is_array(), "{v}");
}

#[test]
fn logs_and_trace_json_output_is_unchanged() {
    let up = Up::start("json-logs");
    let (_, id) = up.hit("/health", Some("g-1"));
    let rows = ok(&up, &["--json", "logs"]);
    assert_eq!(one_json(&rows)["client_id"], "g-1");
    assert_eq!(rows, ok(&up, &["logs", "--json"]));
    assert_eq!(
        one_json(&ok(&up, &["--json", "trace", &id]))["id"],
        id.as_str()
    );
}

#[test]
fn json_mode_keeps_errors_on_stderr() {
    let dir = scratch("json-err");
    let out = bin()
        .args(["status", "--json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("not running"), "{}", stderr(&out));
}

/// Runs `args` in `dir` with `envs` set (remote client mode, E1-E6).
fn remote(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut c = bin();
    c.args(args).current_dir(dir);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().unwrap()
}

/// The control port and token of a running instance, from its `run.json`.
fn port_and_token(up: &Up) -> (u64, String) {
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(up.dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    (
        run["port"].as_u64().unwrap(),
        run["token"].as_str().unwrap().to_string(),
    )
}

/// A port nothing listens on (bound, then released).
fn free_port() -> u16 {
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
fn drain_request(conn: &mut std::net::TcpStream) {
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

#[test]
fn node_down_for_reverts_by_itself() {
    let (up, _dir) = scenario_up("node-down-for");
    let out = ok(&up, &["node", "down", "db", "--for", "3s"]);
    assert!(out.contains("(reverts in 3s)"), "{out}");
    assert!(ok(&up, &["status"]).contains("DOWN"));
    std::thread::sleep(Duration::from_millis(3700));
    assert!(!ok(&up, &["status"]).contains("DOWN"));
}

#[test]
fn node_up_clears_only_the_down_flag() {
    let (up, _dir) = scenario_up("node-up-keeps");
    ok(&up, &["chaos", "set", "db", "--latency", "50ms"]);
    ok(&up, &["node", "down", "db"]);
    assert!(ok(&up, &["status"]).contains("DOWN"));
    ok(&up, &["node", "up", "db"]);
    let st = ok(&up, &["status"]);
    assert!(!st.contains("DOWN") && st.contains("+50ms"), "{st}");
}

#[test]
fn node_up_is_a_noop_exit_0_when_nothing_is_down() {
    let (up, _dir) = scenario_up("node-up-noop");
    ok(&up, &["node", "up", "db"]);
}

#[test]
fn node_up_exits_1_when_a_scenario_holds_it_down() {
    let (up, _dir) = scenario_up("node-up-held");
    ok(&up, &["scenario", "activate", "db-down"]);
    let e = fails(&up, &["node", "up", "db"]);
    assert!(
        e.contains("db is still down: held by an active scenario or override"),
        "{e}"
    );
    assert!(e.contains("kaanal scenario active"), "{e}");
}

#[test]
fn node_down_unknown_node_is_exit_1() {
    let (up, _dir) = scenario_up("node-down-unknown");
    let e = fails(&up, &["node", "down", "nope"]);
    assert!(e.contains("unknown node 'nope'"), "{e}");
}

#[test]
fn chaos_set_for_sends_a_ttl_and_a_bad_value_is_64() {
    let (up, _dir) = scenario_up("chaos-for");
    let out = ok(&up, &["chaos", "set", "db", "--down", "--for", "30s"]);
    assert!(out.contains("(reverts in 30s)"), "{out}");
    // A ttl was sent: the lease runs out by itself.
    ok(&up, &["chaos", "set", "db", "--down", "--for", "3s"]);
    assert!(ok(&up, &["status"]).contains("DOWN"));
    std::thread::sleep(Duration::from_millis(3700));
    assert!(!ok(&up, &["status"]).contains("DOWN"));
    for bad in ["5", "soon", "50ms", "25h"] {
        let e = usage_fails(&up, &["chaos", "set", "db", "--down", "--for", bad]);
        assert!(e.contains("bad --for"), "{bad}: {e}");
        let e = usage_fails(&up, &["node", "down", "db", "--for", bad]);
        assert!(e.contains("bad --for"), "{bad}: {e}");
    }
}

#[test]
fn completions_mention_node() {
    let out = bin().args(["completions", "bash"]).output().unwrap();
    assert!(stdout(&out).contains("node"));
}

#[test]
fn node_list_matches_the_status_table() {
    let up = Up::start("node-list-table");
    ok(&up, &["chaos", "set", "db", "--latency", "300ms"]);
    let status = ok(&up, &["status"]);
    let table: Vec<&str> = status
        .lines()
        .take_while(|l| !l.starts_with("active"))
        .collect();
    assert!(
        table[0].starts_with("NODE") && table[0].contains("STATE"),
        "{status}"
    );
    assert_eq!(
        ok(&up, &["node", "list"]),
        format!("{}\n", table.join("\n"))
    );
}

#[test]
fn node_list_shows_state_and_the_revert_countdown() {
    let up = Up::start("node-list-down");
    ok(&up, &["node", "down", "db", "--for", "30s"]);
    let t = ok(&up, &["node", "list"]);
    let db = t.lines().find(|l| l.starts_with("db")).expect("db row");
    assert_eq!(db.split_whitespace().nth(3), Some("down"), "{t}");
    assert!(db.contains("reverts in"), "{t}");
}

#[test]
fn node_list_json_is_an_array_with_state() {
    let up = Up::start("node-list-json");
    ok(&up, &["node", "down", "db", "--for", "30s"]);
    let rows = one_json(&ok(&up, &["node", "list", "--json"]));
    let db = rows
        .as_array()
        .expect("array")
        .iter()
        .find(|r| r["node"] == "db")
        .expect("db");
    assert_eq!(db["state"], "down");
    assert!(
        db["expires_in_ms"].as_u64().is_some_and(|ms| ms > 0),
        "{db}"
    );
}

#[test]
fn endpoint_flag_may_follow_the_subcommand() {
    let up = Up::start("endpoint-after");
    let (port, token) = port_and_token(&up);
    let elsewhere = scratch("endpoint-after-elsewhere");
    let url = format!("http://127.0.0.1:{port}");
    let envs = [("KAANAL_TOKEN", token.as_str())];
    // Where no subcommand flag collides.
    let out = remote(&elsewhere, &["status", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("postgres"), "{}", stdout(&out));
    // Where the old route-key spelling `--endpoint` also exists, a URL still means the instance (`Cli::fix_route`).
    let out = remote(
        &elsewhere,
        &["chaos", "set", "db", "--down", "--endpoint", &url],
        &envs,
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = remote(&elsewhere, &["logs", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = remote(&elsewhere, &["chaos", "clear", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn old_endpoint_spelling_still_names_a_route() {
    let up = Up::start("route-alias");
    ok(
        &up,
        &["chaos", "set", "--endpoint", "GET /users/{id}", "--down"],
    );
    assert_eq!(up.get_status("/users/42"), 503, "the old spelling works");
    ok(&up, &["chaos", "clear", "--endpoint", "GET /users/{id}"]);
    assert_eq!(up.get_status("/users/42"), 200);
    let out = up.run(&["chaos", "set", "--help"]);
    let help = stdout(&out);
    assert!(help.contains("--route"), "{help}");
}
