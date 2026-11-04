//! `kaanal init`: the OpenAPI-file path (the prompt and render logic has unit tests in `init.rs`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kaanal"));
    c.env_remove("KAANAL_JSON");
    c.env_remove("KAANAL_ENDPOINT");
    c.env_remove("KAANAL_TOKEN");
    c.stdin(Stdio::null());
    c
}

fn spec_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-server/tests/fixtures/openapi.yaml")
}

/// A fresh folder holding a copy of the sample spec as `openapi.yaml`.
fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-init-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(spec_fixture(), d.join("openapi.yaml")).unwrap();
    d
}

fn init(dir: &Path, args: &[&str]) -> Output {
    bin()
        .arg("init")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn init_writes_kaanal_yaml_and_appends_gitignore_once() {
    let dir = project("write");
    std::fs::write(dir.join(".gitignore"), "target").unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes", "--service", "shop"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        out.contains("kaanal.yaml") && out.contains("Next: kaanal up"),
        "{out}"
    );
    let yaml = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    assert!(yaml.contains("spec: \"openapi.yaml\""), "{yaml}");
    assert!(!dir.join("kaanal.yaml.tmp").exists());
    let again = init(&dir, &["openapi.yaml", "--yes", "--force"]);
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert_eq!(ignore, "target\n.kaanal/\n");
}

#[test]
fn init_refuses_an_existing_kaanal_yaml_without_force() {
    let dir = project("exists");
    std::fs::write(dir.join("kaanal.yaml"), "mine\n").unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(stderr(&o).contains("--force"), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        "mine\n"
    );
    assert_eq!(listing(&dir), ["kaanal.yaml", "openapi.yaml"]);
}

#[test]
fn force_overwrites() {
    let dir = project("force");
    std::fs::write(dir.join("kaanal.yaml"), "mine\n").unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes", "--force"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let yaml = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    assert!(yaml.contains("version: 1"), "{yaml}");
}

#[test]
fn a_service_name_with_yaml_metacharacters_is_rejected() {
    for bad in ["a: b", "x\ny: 1", "\"q\"", "Shop", "-x", "a b"] {
        let dir = project("meta");
        let o = init(&dir, &["openapi.yaml", "--yes", "--service", bad]);
        assert_eq!(o.status.code(), Some(64), "{bad}: {}", stderr(&o));
        assert_eq!(listing(&dir), ["openapi.yaml"], "{bad}");
    }
}

#[test]
fn non_interactive_without_a_spec_is_a_usage_error_64() {
    let dir = project("nospec");
    let o = init(&dir, &["--yes"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    let o = init(&dir, &[]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[test]
fn yes_and_non_interactive_never_block() {
    let dir = project("noblock");
    // Piped stdin that never sends a line: a prompt would hang, so reaching exit 0 proves none ran.
    let mut child = bin()
        .args(["init", "openapi.yaml"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(&dir)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("init blocked on stdin");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
    assert!(dir.join("kaanal.yaml").exists());
}

#[test]
fn bad_specs_leave_the_folder_unchanged() {
    let dir = project("bad");
    std::fs::write(
        dir.join("v2.yaml"),
        "swagger: \"2.0\"\ninfo: { title: x, version: \"1\" }\npaths:\n  /a: { get: { responses: { \"200\": { description: ok } } } }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("empty.yaml"),
        "openapi: 3.0.3\ninfo: { title: x, version: \"1\" }\npaths: {}\n",
    )
    .unwrap();
    let before = listing(&dir);
    for spec in ["v2.yaml", "empty.yaml", "missing.yaml"] {
        let o = init(&dir, &[spec, "--yes"]);
        assert_eq!(o.status.code(), Some(1), "{spec}: {}", stderr(&o));
        assert_eq!(listing(&dir), before, "{spec}");
    }
}

#[test]
fn json_prints_the_written_paths() {
    let dir = project("json");
    let o = bin()
        .args(["--json", "init", "openapi.yaml", "--yes"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["written"][0], "kaanal.yaml");
}

#[test]
fn json_with_piped_stdin_prints_exactly_the_object() {
    // No `--yes`: piped stdin is not a terminal, so nothing is asked and stdout is only the JSON.
    let dir = project("json-piped");
    let o = bin()
        .args(["--json", "init", "openapi.yaml"])
        .stdin(Stdio::piped())
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "{\"written\":[\"kaanal.yaml\",\".gitignore\"]}\n"
    );
}

fn yaml_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap()
}

fn validates(dir: &Path) {
    let v = bin().arg("validate").current_dir(dir).output().unwrap();
    assert_eq!(v.status.code(), Some(0), "{}", stderr(&v));
}

#[test]
fn init_writes_real_numbers_and_an_endpoint_per_route() {
    let dir = project("defaults");
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    let yaml = yaml_of(&dir);
    for want in [
        "concurrency: { limit: 256, queue: 100 }, proc: 2ms, timeout: 30s",
        "concurrency: { limit: 32, queue: 100 }, proc: 25ms",
        "net: { p50: 1ms, p99: 3ms }",
        "database: { type: database, flavor: postgres }",
        "cache: { type: cache, flavor: redis }",
        "\"GET /users\": { op: scan, chain: [{ call: database }] }",
        "\"GET /users/{id}\": { op: read, chain: [{ cache: cache }, { call: database }] }",
        "\"POST /orders\": { op: write, chain: [{ call: database }] }",
    ] {
        assert!(yaml.contains(want), "missing `{want}` in\n{yaml}");
    }
    assert_eq!(yaml.matches("net: { p50: 1ms, p99: 3ms }").count(), 3);
    assert!(!yaml.contains("hold") && !yaml.contains("per_kb"), "{yaml}");
    assert!(yaml.matches("estimate").count() >= 2, "{yaml}");
    // one summary line between `Wrote` and `Next`
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "Wrote kaanal.yaml, .gitignore",
            "Chose: event loop, 32 slots, queue 100, 30 s timeout (estimates; edit kaanal.yaml)",
            "Next: kaanal up"
        ],
        "{out}"
    );
}

#[test]
fn thread_per_request_writes_hold_call_and_200_slots() {
    let dir = project("threads");
    let o = init(
        &dir,
        &[
            "openapi.yaml",
            "--yes",
            "--server-style",
            "thread-per-request",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    let yaml = yaml_of(&dir);
    assert!(
        yaml.contains("concurrency: { limit: 200, queue: 100 }, proc: 25ms, hold: call"),
        "{yaml}"
    );
    assert!(!yaml.contains("limit: 32"), "{yaml}");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        out.contains("Chose: thread per request, 200 slots, queue 100, 30 s timeout (estimates; edit kaanal.yaml)"),
        "{out}"
    );
}

#[test]
fn a_bad_server_style_flag_is_refused_and_names_the_values() {
    let dir = project("badstyle");
    let o = init(&dir, &["openapi.yaml", "--yes", "--server-style", "fibers"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("event-loop or thread-per-request"),
        "{}",
        stderr(&o)
    );
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[test]
fn json_output_keeps_its_shape_with_a_style_flag() {
    let dir = project("json-style");
    let o = bin()
        .args([
            "--json",
            "init",
            "openapi.yaml",
            "--yes",
            "--server-style",
            "thread-per-request",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "{\"written\":[\"kaanal.yaml\",\".gitignore\"]}\n"
    );
}

const ODD: &str = "openapi: 3.0.3
info: { title: odd, version: \"1\" }
paths:
  /users:
    get: { responses: { \"200\": { description: ok } } }
    post: { responses: { \"201\": { description: ok } } }
    head: { responses: { \"200\": { description: ok } } }
    options: { responses: { \"200\": { description: ok } } }
  /users/{id}:
    get: { parameters: [ { name: id, in: path, required: true, schema: { type: string } } ], responses: { \"200\": { description: ok } } }
    put: { parameters: [ { name: id, in: path, required: true, schema: { type: string } } ], responses: { \"200\": { description: ok } } }
    patch: { parameters: [ { name: id, in: path, required: true, schema: { type: string } } ], responses: { \"200\": { description: ok } } }
    delete: { parameters: [ { name: id, in: path, required: true, schema: { type: string } } ], responses: { \"204\": { description: ok } } }
  /orders/{oid}/items:
    get: { parameters: [ { name: oid, in: path, required: true, schema: { type: string } } ], responses: { \"200\": { description: ok } } }
  /reports/:
    get: { responses: { \"200\": { description: ok } } }
";

#[test]
fn the_endpoints_block_follows_the_method_and_validates_for_odd_specs() {
    let dir = project("odd");
    std::fs::write(dir.join("odd.yaml"), ODD).unwrap();
    let o = init(&dir, &["odd.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    let yaml = yaml_of(&dir);
    let (read, scan, write) = (
        "op: read, chain: [{ cache: cache }, { call: database }]",
        "op: scan, chain: [{ call: database }]",
        "op: write, chain: [{ call: database }]",
    );
    for (key, op) in [
        ("GET /users", scan),
        ("POST /users", write),
        ("GET /users/{id}", read),
        ("PUT /users/{id}", write),
        ("PATCH /users/{id}", write),
        ("DELETE /users/{id}", write),
        ("GET /orders/{oid}/items", scan),
        ("GET /reports/", scan),
    ] {
        let line = yaml
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("\"{key}\":")))
            .unwrap_or_else(|| panic!("no entry for {key}:\n{yaml}"));
        assert!(line.contains(op), "{key}: {line}");
    }
    // HEAD and OPTIONS get no entry of their own
    assert!(
        !yaml.contains("\"HEAD") && !yaml.contains("\"OPTIONS"),
        "{yaml}"
    );
    // every route of the spec that has an entry is listed once, none is left over
    assert_eq!(yaml.matches("op: ").count(), 8, "{yaml}");
    // and `kaanal endpoints` has only the HEAD and OPTIONS routes left to offer
    let e = bin().arg("endpoints").current_dir(&dir).output().unwrap();
    let shown = String::from_utf8_lossy(&e.stdout).into_owned();
    assert_eq!(shown.matches(": {}").count(), 2, "{shown}");
    assert!(shown.contains("HEAD /users"), "{shown}");
    assert!(shown.contains("OPTIONS /users"), "{shown}");
}

#[test]
fn a_spec_of_only_head_and_options_gets_a_valid_file_with_no_endpoints_block() {
    let dir = project("headonly");
    std::fs::write(
        dir.join("h.yaml"),
        "openapi: 3.0.3\ninfo: { title: h, version: \"1\" }\npaths:\n  /ping:\n    head: { responses: { \"200\": { description: ok } } }\n    options: { responses: { \"200\": { description: ok } } }\n",
    )
    .unwrap();
    let o = init(&dir, &["h.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    assert!(!yaml_of(&dir).contains("endpoints:"));
}

/// The generated `peak` profile of `dir`, read back through the config loader.
fn peak_of(dir: &Path) -> kaanal_core::config::TrafficConfig {
    let cfg = kaanal_core::Config::from_yaml(&yaml_of(dir)).unwrap();
    cfg.traffic["peak"].clone()
}

#[test]
fn init_writes_a_peak_profile_with_one_journey_per_endpoint() {
    let dir = project("peak");
    std::fs::write(dir.join("odd.yaml"), ODD).unwrap();
    let o = init(&dir, &["odd.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    let yaml = yaml_of(&dir);
    assert!(
        yaml.contains("\ntraffic:\n  peak:\n    rps: 100\n    duration: 30s\n"),
        "{yaml}"
    );
    assert!(
        yaml.contains("# traffic:") && yaml.contains("An estimate"),
        "{yaml}"
    );
    let p = peak_of(&dir);
    assert_eq!((p.rps, p.duration), (100.0, Some(Duration::from_secs(30))));
    let mut got: Vec<(&str, f64)> = p
        .journeys
        .iter()
        .map(|j| (j.endpoint.as_str(), j.weight))
        .collect();
    got.sort_by(|a, b| a.0.cmp(b.0));
    // the endpoints: keys; HEAD and OPTIONS have none
    let mut want = [
        ("GET /users", 3.0),
        ("POST /users", 1.0),
        ("GET /users/{id}", 3.0),
        ("PUT /users/{id}", 1.0),
        ("PATCH /users/{id}", 1.0),
        ("DELETE /users/{id}", 1.0),
        ("GET /orders/{oid}/items", 3.0),
        ("GET /reports/", 3.0),
    ];
    want.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(got, want);
}

#[test]
fn the_generated_peak_profile_runs_in_the_simulator() {
    use kaanal_core::chain::{EndpointsInput, compile};
    use kaanal_core::{ChaosState, Config, endpoint_key};
    use kaanal_sim::{SimConfig, Stream, run, summarize};
    let dir = project("peak-run");
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let p = peak_of(&dir);
    let cfg = Config::from_yaml(&yaml_of(&dir)).unwrap();
    let (topo, routes) = kaanal_server::spec_routes(&cfg, &dir).unwrap();
    let eps = compile(&topo, &routes, EndpointsInput::inline(&cfg)).unwrap();
    let total: f64 = p.journeys.iter().map(|j| j.weight).sum();
    let sim = SimConfig {
        streams: p
            .journeys
            .iter()
            .map(|j| Stream {
                endpoint_key: endpoint_key(&j.endpoint).unwrap(),
                rate_per_s: p.rps * j.weight / total,
                req_bytes: 0,
                resp_bytes: 0,
            })
            .collect(),
        duration: Duration::from_secs(10),
        warmup: Duration::from_secs(1),
        seed: 42,
    };
    let rep = run(&topo, &eps, &ChaosState::default(), &sim).unwrap();
    let s = summarize(&topo, &eps, &sim, &rep);
    assert!(
        s.requests > 0 && s.failed == 0,
        "{} / {}",
        s.requests,
        s.failed
    );
}

#[test]
fn a_head_and_options_only_spec_gets_no_traffic_section() {
    let dir = project("peak-none");
    std::fs::write(
        dir.join("h.yaml"),
        "openapi: 3.0.3\ninfo: { title: h, version: \"1\" }\npaths:\n  /ping:\n    head: { responses: { \"200\": { description: ok } } }\n    options: { responses: { \"200\": { description: ok } } }\n",
    )
    .unwrap();
    let o = init(&dir, &["h.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    validates(&dir);
    assert!(!yaml_of(&dir).contains("traffic:"));
}

/// p50 of one request a second through the generated file, from the simulator.
fn sim_p50_ms(dir: &Path, key: &str) -> f64 {
    use kaanal_core::chain::{EndpointsInput, compile};
    use kaanal_core::{ChaosState, Config};
    use kaanal_sim::{SimConfig, Stream, run, summarize};
    let cfg = Config::from_yaml(&yaml_of(dir)).unwrap();
    let (topo, routes) = kaanal_server::spec_routes(&cfg, dir).unwrap();
    let eps = compile(&topo, &routes, EndpointsInput::inline(&cfg)).unwrap();
    let sim = SimConfig {
        streams: vec![Stream {
            endpoint_key: key.into(),
            rate_per_s: 1.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(60),
        warmup: Duration::ZERO,
        seed: 42,
    };
    let rep = run(&topo, &eps, &ChaosState::default(), &sim).unwrap();
    summarize(&topo, &eps, &sim, &rep).latency.p50_ms
}

#[test]
fn the_defaults_make_a_request_take_real_time() {
    let dir = project("sim");
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    // 2 ms gateway + 25 ms service + 1 ms nets + a database read: well over the old 0 ms
    let read = sim_p50_ms(&dir, "GET /users/{id}");
    assert!(read > 20.0, "GET by id p50 {read} ms");
    let write = sim_p50_ms(&dir, "POST /orders");
    assert!(write > 20.0, "POST p50 {write} ms");
}

/// A `kaanal up` child, killed on drop.
struct Up(Child);

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn init_output_validates_and_starts() {
    let dir = project("starts");
    let o = init(&dir, &["openapi.yaml", "--yes", "--locale", "en-US"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let v = bin().arg("validate").current_dir(&dir).output().unwrap();
    assert_eq!(v.status.code(), Some(0), "{}", stderr(&v));

    let _up = Up(bin()
        .args(["up", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(&dir)
        .spawn()
        .unwrap());
    let run = dir.join(".kaanal/run.json");
    let deadline = Instant::now() + Duration::from_secs(20);
    let port = loop {
        if let Some(p) = std::fs::read_to_string(&run)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map(|j| j["port"].clone())
        {
            break p;
        }
        assert!(
            Instant::now() < deadline,
            "kaanal up did not write run.json"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let status = agent
        .get(&format!("http://127.0.0.1:{port}/users"))
        .call()
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 200);

    let e = bin().arg("endpoints").current_dir(&dir).output().unwrap();
    assert_eq!(e.status.code(), Some(0), "{}", stderr(&e));
    // init wrote an entry for every route, so there is nothing left to add
    assert!(String::from_utf8_lossy(&e.stdout).contains("nothing to add"));
}

#[test]
fn an_unusable_gitignore_is_a_warning_and_init_still_succeeds() {
    // A directory, and a UTF-16 file that must stay byte-identical.
    let dir = project("ignore-dir");
    std::fs::create_dir(dir.join(".gitignore")).unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("could not update .gitignore"),
        "{}",
        stderr(&o)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("Wrote kaanal.yaml"));
    assert!(dir.join("kaanal.yaml").exists());

    let dir = project("ignore-utf16");
    let utf16 = [0xff, 0xfe, b'a', 0, b'\n', 0];
    std::fs::write(dir.join(".gitignore"), utf16).unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("add .kaanal/ yourself"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(dir.join(".gitignore")).unwrap(), utf16);
}

#[test]
fn a_crlf_gitignore_keeps_crlf() {
    let dir = project("crlf");
    std::fs::write(dir.join(".gitignore"), "target\r\n").unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        std::fs::read(dir.join(".gitignore")).unwrap(),
        b"target\r\n.kaanal/\r\n"
    );
}

#[test]
fn an_existing_temp_file_is_refused_and_left_alone() {
    let dir = project("tmp");
    std::fs::write(dir.join("kaanal.yaml.tmp"), "theirs\n").unwrap();
    let o = init(&dir, &["openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(stderr(&o).contains("interrupted init"), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml.tmp")).unwrap(),
        "theirs\n"
    );
    assert_eq!(listing(&dir), ["kaanal.yaml.tmp", "openapi.yaml"]);
}

#[test]
fn errors_name_kaanal_yaml_not_the_temp_file() {
    let dir = project("names");
    // Passes the tag shape, but no such pack exists: `check` rejects it.
    let o = init(&dir, &["openapi.yaml", "--yes", "--locale", "zz-ZZ"]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let e = stderr(&o);
    assert!(e.contains("kaanal.yaml") && !e.contains(".tmp"), "{e}");
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

/// A loopback HTTP server: connection `i` gets `answer(port, i)` and is closed. The thread lives
/// until the test process exits.
fn serve(answer: impl Fn(u16, usize) -> Vec<u8> + Send + 'static) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for (i, s) in l.incoming().enumerate() {
            let Ok(mut s) = s else { return };
            let (mut head, mut b) = (Vec::new(), [0u8]);
            while !head.ends_with(b"\r\n\r\n") && s.read(&mut b).is_ok_and(|n| n == 1) {
                head.push(b[0]);
            }
            let _ = s.write_all(&answer(port, i));
        }
    });
    port
}

fn reply(status: &str, extra: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn spec_text() -> String {
    std::fs::read_to_string(spec_fixture()).unwrap()
}

#[test]
fn fetches_a_spec_over_http_loopback() {
    let dir = project("fetch");
    let spec = spec_text();
    let port = serve(move |_, _| reply("200 OK", "", &spec));
    let url = format!("http://127.0.0.1:{port}/v1/shop.yaml?key=k");
    let o = init(&dir, &[&url, "--yes"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("shop.yaml")).unwrap(),
        spec_text()
    );
    let yaml = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    assert!(yaml.contains("spec: \"shop.yaml\""), "{yaml}");
    assert!(String::from_utf8_lossy(&o.stdout).contains("shop.yaml"));
    let v = bin().arg("validate").current_dir(&dir).output().unwrap();
    assert_eq!(v.status.code(), Some(0), "{}", stderr(&v));
}

#[test]
fn rejects_http_to_a_non_loopback_host() {
    let dir = project("remote");
    for url in [
        "http://example.invalid/openapi.yaml",
        "http://10.0.0.1/a.yaml",
        "ftp://127.0.0.1/a.yaml",
    ] {
        let o = init(&dir, &[url, "--yes"]);
        assert_eq!(o.status.code(), Some(64), "{url}: {}", stderr(&o));
        assert_eq!(listing(&dir), ["openapi.yaml"], "{url}");
    }
}

#[test]
fn follows_up_to_three_redirects_and_stops_at_the_fourth() {
    for (hops, code) in [(3, 0), (4, 1)] {
        let dir = project(&format!("redirects-{hops}"));
        let spec = spec_text();
        let port = serve(move |port, i| match i {
            // One absolute location, then relative ones, each resolved against the last hop.
            0 => reply(
                "302 Found",
                &format!("Location: http://127.0.0.1:{port}/r/1\r\n"),
                "",
            ),
            i if i < hops => reply("301 Moved", &format!("Location: {}\r\n", i + 1), ""),
            _ => reply("200 OK", "", &spec),
        });
        let o = init(
            &dir,
            &[&format!("http://127.0.0.1:{port}/api.yaml"), "--yes"],
        );
        assert_eq!(o.status.code(), Some(code), "{hops}: {}", stderr(&o));
        if code == 0 {
            assert!(dir.join("api.yaml").exists());
        } else {
            assert!(stderr(&o).contains("redirects"), "{}", stderr(&o));
            assert_eq!(listing(&dir), ["openapi.yaml"]);
        }
    }
}

#[test]
fn rejects_html_that_is_not_openapi() {
    let dir = project("html");
    let port = serve(|_, _| {
        reply(
            "200 OK",
            "Content-Type: text/html\r\n",
            "<html><body>hi</body></html>",
        )
    });
    let o = init(
        &dir,
        &[&format!("http://127.0.0.1:{port}/spec.json"), "--yes"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("not an OpenAPI 3.0 document"),
        "{}",
        stderr(&o)
    );
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[test]
fn error_text_names_the_url_not_the_body() {
    for status in ["200 OK", "500 Internal Server Error"] {
        let dir = project("errtext");
        let port = serve(move |_, _| reply(status, "", "SECRET-BODY\x1b[31m"));
        let url = format!("http://127.0.0.1:{port}/x/api.yaml?token=SECRET-QUERY");
        let o = init(&dir, &[&url, "--yes"]);
        let e = stderr(&o);
        assert_eq!(o.status.code(), Some(1), "{e}");
        assert!(
            e.contains(&format!("http://127.0.0.1:{port}/x/api.yaml")),
            "{e}"
        );
        assert!(!e.contains("SECRET") && !e.contains('\x1b'), "{e}");
        assert_eq!(listing(&dir), ["openapi.yaml"]);
    }
}

#[test]
fn a_failed_check_removes_the_downloaded_spec() {
    let dir = project("failcheck");
    let port = serve(|_, _| {
        reply(
            "200 OK",
            "",
            "openapi: 3.0.3\ninfo: { title: x, version: \"1\" }\npaths: {}\n",
        )
    });
    let o = init(
        &dir,
        &[&format!("http://127.0.0.1:{port}/empty.yaml"), "--yes"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(stderr(&o).contains("no paths"), "{}", stderr(&o));
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[test]
fn url_with_userinfo_is_a_usage_error() {
    let dir = project("userinfo");
    for url in [
        "http://u:p@127.0.0.1:1/a.yaml",
        "https://u@example.com/a.yaml",
    ] {
        let o = init(&dir, &[url, "--yes"]);
        let e = stderr(&o);
        assert_eq!(o.status.code(), Some(64), "{url}: {e}");
        assert!(e.contains("user") && !e.contains("u:p@"), "{e}");
    }
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[test]
fn refuses_to_overwrite_an_existing_spec_without_force() {
    let dir = project("overwrite");
    let mine = std::fs::read(dir.join("openapi.yaml")).unwrap();
    let port = serve(|_, _| reply("200 OK", "", &spec_text().replace("Sample app", "Theirs")));
    let url = format!("http://127.0.0.1:{port}/openapi.yaml");
    let o = init(&dir, &[&url, "--yes"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(stderr(&o).contains("--force"), "{}", stderr(&o));
    // Forced, but the project fails `check` (no such locale pack): the old file comes back from
    // its backup and no backup is left.
    let o = init(&dir, &[&url, "--yes", "--force", "--locale", "zz-ZZ"]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
    assert_eq!(std::fs::read(dir.join("openapi.yaml")).unwrap(), mine);
    assert_eq!(listing(&dir), ["openapi.yaml"]);
    // A leftover backup (an interrupted forced init) is refused and kept.
    std::fs::write(dir.join("openapi.yaml.kaanal-old"), "older").unwrap();
    let o = init(&dir, &[&url, "--yes", "--force"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("openapi.yaml.kaanal-old"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(dir.join("openapi.yaml")).unwrap(), mine);
    std::fs::remove_file(dir.join("openapi.yaml.kaanal-old")).unwrap();
    let o = init(&dir, &[&url, "--yes", "--force"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("Replaced the existing openapi.yaml"),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(!dir.join("openapi.yaml.kaanal-old").exists());
    assert!(
        std::fs::read_to_string(dir.join("openapi.yaml"))
            .unwrap()
            .contains("Theirs")
    );
}

#[test]
fn a_loopback_fetch_ignores_environment_proxies() {
    let dir = project("proxy");
    let spec = spec_text();
    let port = serve(move |_, _| reply("200 OK", "", &spec));
    let dead = "http://127.0.0.1:9"; // discard port: nothing listens
    let o = bin()
        .args(["init", &format!("http://127.0.0.1:{port}/p.yaml"), "--yes"])
        .env("HTTP_PROXY", dead)
        .env("http_proxy", dead)
        .env("ALL_PROXY", dead)
        .env("all_proxy", dead)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(dir.join("p.yaml").exists());
}

#[test]
fn a_redirect_to_a_forbidden_host_is_refused() {
    let dir = project("forbidden");
    let port = serve(|_, _| {
        reply(
            "302 Found",
            "Location: http://example.invalid/openapi.yaml\r\n",
            "",
        )
    });
    let o = init(&dir, &[&format!("http://127.0.0.1:{port}/a.yaml"), "--yes"]);
    let e = stderr(&o);
    assert_eq!(o.status.code(), Some(1), "{e}");
    assert!(
        e.contains("redirect refused") && e.contains("http://example.invalid/openapi.yaml"),
        "{e}"
    );
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}

#[cfg(not(feature = "https"))]
#[test]
fn without_the_https_feature_an_https_url_is_a_usage_error() {
    let dir = project("nohttps");
    let o = init(&dir, &["https://example.com/openapi.yaml", "--yes"]);
    assert_eq!(o.status.code(), Some(64), "{}", stderr(&o));
    assert!(stderr(&o).contains("'https' feature"), "{}", stderr(&o));
    assert_eq!(listing(&dir), ["openapi.yaml"]);
}
