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
        let dir = scratch(name);
        let child = bin()
            .args(["up", "--port", "0"])
            .args(extra)
            .arg("-c")
            .arg(fixture())
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
            "Seed:  {n} (determinism: realistic; replay with determinism: controlled and seed: {n})"
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
