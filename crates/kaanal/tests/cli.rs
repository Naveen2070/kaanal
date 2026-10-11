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
