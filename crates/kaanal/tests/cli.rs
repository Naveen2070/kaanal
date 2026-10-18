use sha2::Digest as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kaanal"))
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
    let out = up.run(&["chaos", "set", "--endpoint", "get /users/{id}", "--down"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(up.get_status("/users/42"), 503, "the endpoint is down");
    let text = String::from_utf8_lossy(&up.run(&["status"]).stdout).into_owned();
    assert!(
        text.contains("endpoint GET /users/{id}:") && text.contains("DOWN"),
        "{text}"
    );
    let out = up.run(&["chaos", "clear", "--endpoint", "GET /users/{id}"]);
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

/// Runs `args`, asserts exit 1 and returns stderr.
fn fails(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stdout(&out));
    stderr(&out)
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
        let e = fails(&up, &["scenario", "show", reserved]);
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
    let e = fails(&up, &["scenario", "load", badname.to_str().unwrap()]);
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
    let e = fails(&up, &["scenario", "save", "x", "--from", "mine"]);
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
    ok(&up, &["chaos", "set", "--endpoint", ep, "--latency", "9ms"]);
    ok(
        &up,
        &["chaos", "set", "db", "--endpoint", ep, "--latency", "11ms"],
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
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
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
fn bad_arguments_exit_1_not_2() {
    // 2 means "not running" (R6.3); clap's default for usage errors is 2.
    for args in [&["chaos", "set", "db", "--error", "70000"][..], &["nope"]] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
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
    let e = fails(&up, &["scenario", "activate", "db-down", "--ttl", "5"]);
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
    let e = fails(&up, &["cycle", "GET /users", "--state", "broken"]);
    assert!(e.contains("loading") && e.contains("success"), "{e}");
    let e = fails(
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
            "--endpoint",
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
    let r = ok(&up, &["logs", "--endpoint", "GET /users/{id}"]);
    assert!(r.lines().count() == 1 && r.contains("503"), "{r}");
    assert_eq!(ok(&up, &["logs", "--scope", "nobody"]), "");
    assert!(fails(&up, &["logs", "--status", "9xx"]).contains("status"));
    assert!(fails(&up, &["logs", "--scope", ""]).contains("scope"));
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
    assert!(fails(&up, &["logs", "--since", "yesterday"]).contains("since"));
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
            "--endpoint",
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
        let e = fails(&up, &["logs", "-n", n]);
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
            "--endpoint",
            "GET /users/{id}",
            "--error-rate",
            "100%",
            "--error",
            "503",
        ],
    );
    up.hit("/users/2", Some("pre-f"));
    // The tail is printed after the subscription, so a tail line means the follower is live.
    let by_endpoint = Follow::start(&up, &["--endpoint", "GET /users/{id}"]);
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
