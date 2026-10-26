//! Shared helpers: the binary, scratch folders, the `Up` child process.

use sha2::Digest as _;

use std::io::Read;

use std::path::{Path, PathBuf};

use std::process::{Child, Command, Output, Stdio};

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub(crate) fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kaanal"));
    c.env_remove("KAANAL_JSON");
    c.env_remove("KAANAL_ENDPOINT");
    c.env_remove("KAANAL_TOKEN");
    c
}

pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-server/tests/fixtures")
}

pub(crate) fn fixture() -> PathBuf {
    fixtures().join("kaanal.yaml")
}

pub(crate) fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub(crate) fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A `kaanal up` child on an OS-chosen port, killed on drop.
pub(crate) struct Up {
    pub(crate) child: Child,
    pub(crate) dir: PathBuf,
}

impl Up {
    pub(crate) fn start(name: &str) -> Up {
        Up::start_with(name, &[])
    }

    pub(crate) fn start_with(name: &str, extra: &[&str]) -> Up {
        Up::launch(scratch(name), extra, &fixture())
    }

    /// Starts in an existing project dir with its own `kaanal.yaml`.
    pub(crate) fn start_in(dir: PathBuf) -> Up {
        Up::launch(dir, &[], Path::new("kaanal.yaml"))
    }

    pub(crate) fn launch(dir: PathBuf, extra: &[&str], cfg: &Path) -> Up {
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

    pub(crate) fn run(&self, args: &[&str]) -> Output {
        bin().args(args).current_dir(&self.dir).output().unwrap()
    }

    /// The status a plain GET of `path` gets from the running mock.
    pub(crate) fn get_status(&self, path: &str) -> u16 {
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

/// Runs `cmd` with both pipes read to EOF, failing if EOF does not come within 30 s: a daemon that
/// inherited the pipes would hold them open forever.
pub(crate) fn piped(mut cmd: Command) -> Output {
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

pub(crate) fn health(port: u16) -> u16 {
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

/// Kills (and reaps) a helper process on drop, so a failing assert never leaks it.
pub(crate) struct Helper(pub(crate) Child);

/// A project dir: the fixture spec and a `kaanal.yaml` of gateway, app and db (8 lines),
/// then `tail` from line 9.
pub(crate) fn project(name: &str, tail: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::copy(fixtures().join("openapi.yaml"), dir.join("openapi.yaml")).unwrap();
    std::fs::write(
        dir.join("kaanal.yaml"),
        format!("version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml }}\n  db: {{ type: database, flavor: postgres }}\nedges:\n  - {{ from: gw, to: app }}\n  - {{ from: app, to: db }}\n{tail}"),
    )
    .unwrap();
    dir
}

pub(crate) fn kaanal(dir: &Path, args: &[&str]) -> Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

pub(crate) fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Parses stdout that must be exactly one JSON value on one line.
pub(crate) fn one_json(out: &str) -> serde_json::Value {
    assert_eq!(out.trim_end().lines().count(), 1, "{out}");
    serde_json::from_str(out).unwrap_or_else(|e| panic!("not JSON ({e}): {out}"))
}

/// An auth-only project (one user) and its private `KAANAL_CONFIG_DIR`; `seed` writes the
/// shared test key where kaanal looks for it.
pub(crate) fn auth_project(name: &str, seed: bool) -> (PathBuf, PathBuf) {
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

pub(crate) fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn run_json(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap()).unwrap()
}

/// Waits (bounded) for `dir`'s run.json to carry `pid`, failing early if the child exits.
pub(crate) fn wait_for_pid(dir: &Path, child: &mut Child) {
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
pub(crate) fn wait_exit(child: &mut Child, secs: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s.code();
        }
        assert!(Instant::now() < deadline, "up did not exit in {secs}s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A lock held by this test process, standing in for a running (or still starting) `up`.
pub(crate) fn hold_lock(dir: &Path) -> std::fs::File {
    std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
    let f = std::fs::File::create(dir.join(".kaanal/lock")).unwrap();
    f.try_lock().unwrap();
    f
}

/// Runs `args` in `dir` with `envs` set (remote client mode, E1-E6).
pub(crate) fn remote(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut c = bin();
    c.args(args).current_dir(dir);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().unwrap()
}

/// The control port and token of a running instance, from its `run.json`.
pub(crate) fn port_and_token(up: &Up) -> (u64, String) {
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(up.dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    (
        run["port"].as_u64().unwrap(),
        run["token"].as_str().unwrap().to_string(),
    )
}

/// A running instance in a project dir with one inline scenario, `db-down`.
pub(crate) fn scenario_up(name: &str) -> (Up, PathBuf) {
    let dir = project(
        name,
        "scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n",
    );
    (Up::start_in(dir.clone()), dir)
}

/// Runs `args`, asserts exit 0 and returns stdout.
pub(crate) fn ok(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

/// Runs `args`, asserts exit 64 (usage error) and returns stderr.
pub(crate) fn usage_fails(up: &Up, args: &[&str]) -> String {
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
pub(crate) fn fails(up: &Up, args: &[&str]) -> String {
    let out = up.run(args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stdout(&out));
    stderr(&out)
}

/// Asserts `e` holds `file:LINE:COL` for `file`.
pub(crate) fn assert_located(e: &str, file: &str) {
    let ok = e.match_indices(&format!("{file}:")).any(|(i, _)| {
        let rest = &e[i + file.len() + 1..];
        let (line, tail) = rest.split_at(rest.find(':').unwrap_or(0));
        !line.is_empty()
            && line.chars().all(|c| c.is_ascii_digit())
            && tail[1..].starts_with(|c: char| c.is_ascii_digit())
    });
    assert!(ok, "no {file}:line:col in {e}");
}

pub(crate) fn spawn_up(dir: &Path) -> Child {
    bin()
        .args(["up", "--port", "0", "-c"])
        .arg(fixture())
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

pub(crate) fn run_json_pid(dir: &Path) -> Option<u64> {
    let s = std::fs::read_to_string(dir.join(".kaanal/run.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&s).ok()?["pid"].as_u64()
}

pub(crate) fn assert_64(args: &[&str]) {
    let (code, err) = exit_of(args);
    assert_eq!(code, Some(64), "{args:?}: {err}");
}

/// Reads both pipes of `child` to EOF on threads; the receiver gets `(0|1, bytes)` per pipe.
pub(crate) fn drain_pipes(child: &mut Child) -> std::sync::mpsc::Receiver<(usize, Vec<u8>)> {
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

/// One PKCS#8 key per test binary, pre-seeded where a test needs a key (spec section 7: only
/// `up_generates_the_key_once_then_reuses_it` lets kaanal generate one).
pub(crate) fn test_pem() -> &'static str {
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
pub(crate) fn key_path(cfg: &Path, project: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(project).unwrap();
    let text = canon.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let id = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
    cfg.join("kaanal")
        .join("keys")
        .join(format!("{}.pem", &id[..16]))
}

pub(crate) fn exit_of(args: &[&str]) -> (Option<i32>, String) {
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

/// Runs the CLI in a fresh scratch dir (no instance running) and returns the exit code and stderr.
pub(crate) static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
