//! Shared helpers: the binary, project folders, the `Up` child process, the HTTP agent.

use crate::demo;
use std::path::{Path, PathBuf};

use std::process::{Child, Command, Output, Stdio};

use std::time::{Duration, Instant};

pub(crate) fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kaanal"));
    c.env_remove("KAANAL_JSON");
    c.env_remove("KAANAL_ENDPOINT");
    c.env_remove("KAANAL_TOKEN");
    c.stdin(Stdio::null());
    c
}

/// A fresh folder holding the template.
pub(crate) fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-demo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for (path, text) in demo::FILES {
        let p = d.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    d
}

pub(crate) fn run(dir: &Path, args: &[&str]) -> Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

/// A `kaanal up` child, killed on drop, then its folder removed.
pub(crate) struct Up(pub(crate) Child, pub(crate) PathBuf);

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

pub(crate) fn up(dir: &Path) -> (Up, u16) {
    let child = bin()
        .args(["up", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.join("up.err")).unwrap())
        .current_dir(dir)
        .spawn()
        .unwrap();
    let mut guard = Up(child, dir.to_path_buf());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(p) = std::fs::read_to_string(dir.join(".kaanal/run.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|j| j["port"].as_u64())
        {
            return (guard, u16::try_from(p).unwrap());
        }
        let exited = guard.0.try_wait().unwrap();
        assert!(
            exited.is_none() && Instant::now() < deadline,
            "kaanal up did not start ({exited:?}): {}",
            std::fs::read_to_string(dir.join("up.err")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// Removes a folder on drop, so a failing test still cleans up.
pub(crate) struct Cleanup(pub(crate) PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The stories (scenario files) the template ships, in the order `kaanal` lists them.
pub(crate) const STORIES: [&str; 15] = [
    "auth-errors",
    "black-friday",
    "brownout",
    "cache-down",
    "cart-write-errors",
    "db-timeout",
    "flaky-network",
    "gateway-down",
    "meltdown",
    "payment-declined",
    "payment-down",
    "payment-flaky",
    "payment-slow",
    "payment-timeout",
    "slow-db",
];

/// An empty folder to run `kaanal demo` in; `name` may hold spaces and non-ASCII.
pub(crate) fn empty(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-demo-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The names in `dir`, sorted.
pub(crate) fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

pub(crate) fn demo_in(dir: &Path, args: &[&str]) -> Demo {
    demo_with(dir, &[], args)
}

pub(crate) fn status_of(url: &str) -> u16 {
    agent().get(url).call().unwrap().status().as_u16()
}

/// The token of the instance running in `dir`.
pub(crate) fn run_token(dir: &Path) -> String {
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    run["token"].as_str().unwrap().to_string()
}

/// A link from `link` to `target`: a symlink on Unix, a junction on Windows (no privilege needed).
pub(crate) fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

pub(crate) fn get_json(agent: &ureq::Agent, url: &str) -> serde_json::Value {
    agent
        .get(url)
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap()
}

/// The embedded text of a site file (`site/` + `path`).
pub(crate) fn site_file(path: &str) -> &'static str {
    let want = format!("site/{path}");
    demo::FILES
        .iter()
        .find(|(p, _)| *p == want)
        .map_or_else(|| panic!("{want} is not embedded"), |(_, t)| *t)
}

/// The quoted values after each `key` (`src="`, `scenario: "`, ...) in `text`.
pub(crate) fn values_after<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.match_indices(key)
        .filter_map(|(i, _)| {
            let rest = &text[i + key.len()..];
            rest.find('"').map(|end| &rest[..end])
        })
        .collect()
}

/// A running `kaanal demo`: its banner (stdout up to the `Stop:` line, or the JSON object) and
/// its port. Killed on drop, then its folder and its stderr file removed.
pub(crate) struct Demo {
    pub(crate) child: Child,
    pub(crate) dir: PathBuf,
    pub(crate) err: PathBuf,
    pub(crate) out: String,
    pub(crate) port: u16,
}

/// Starts `kaanal [pre...] demo [args...]` in `dir` and reads its banner. Its stderr goes to a
/// file beside `dir`, not in it.
pub(crate) fn demo_with(dir: &Path, pre: &[&str], args: &[&str]) -> Demo {
    use std::io::BufRead;
    let err = PathBuf::from(format!("{}.err", dir.display()));
    let mut child = bin()
        .args(pre)
        .arg("demo")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&err).unwrap())
        .current_dir(dir)
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut d = Demo {
        child,
        dir: dir.to_path_buf(),
        err,
        out: String::new(),
        port: 0,
    };
    for line in lines.by_ref() {
        let line = line.unwrap();
        d.out.push_str(&line);
        d.out.push('\n');
        if let Some(rest) = line.strip_prefix("Demo:") {
            let rest = rest.trim().strip_prefix("http://localhost:").unwrap();
            d.port = rest[..rest.find('/').unwrap()].parse().unwrap();
        }
        if line.starts_with('{') {
            let j: serde_json::Value = serde_json::from_str(&line).unwrap();
            d.port = u16::try_from(j["port"].as_u64().unwrap()).unwrap();
            break;
        }
        if line.starts_with("Stop:") {
            break;
        }
    }
    assert!(d.port != 0, "demo did not start: {}{}", d.out, d.stderr());
    // Keep the pipe drained so the demo never blocks on a write.
    std::thread::spawn(move || lines.for_each(drop));
    d
}
