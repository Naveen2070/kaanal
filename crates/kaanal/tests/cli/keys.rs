//! The auth signing key: generation, reuse, corruption, permissions.

use crate::common::*;

use std::io::{BufRead, BufReader};

use std::path::Path;

use std::process::{Child, Output, Stdio};

#[cfg(windows)]
use std::process::Command;

/// `kaanal up` in `project` with `KAANAL_CONFIG_DIR=cfg`: the child and its stdout lines up
/// to the `Auth:` line (or all of them, if it exits first).
pub(crate) fn up_auth(project: &Path, cfg: &Path) -> (Child, Vec<String>) {
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
pub(crate) fn up_fails(project: &Path, cfg: &Path) -> Output {
    bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(project)
        .env("KAANAL_CONFIG_DIR", cfg)
        .output()
        .unwrap()
}

pub(crate) fn port_of(lines: &[String]) -> u16 {
    lines
        .iter()
        .find_map(|l| l.strip_prefix("Mocks: http://localhost:"))
        .unwrap()
        .parse()
        .unwrap()
}

/// The `kid` the running instance serves in its JWKS.
pub(crate) fn served_kid(port: u16) -> serde_json::Value {
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
