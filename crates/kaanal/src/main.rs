//! The `kaanal` binary: CLI, process lifecycle, and a thin HTTP client for the
//! control API.
//!
//! `up`, `validate`, `schema` and `endpoints` work locally; `status` and `chaos` find the running
//! instance via `.kaanal/run.json` and call its control API (rule R2.6: every
//! interface is a thin client, no per-interface logic). `up` loads (or creates) the auth
//! signing key only when the config has an auth node (`keys`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use kaanal_core::chain::skeleton;
use kaanal_core::config::{NodeConfig, parse_endpoints};
use kaanal_core::diagnostics::Located;
use kaanal_core::{Config, ConfigError, EndpointsConfig};
use kaanal_server::ServeOpts;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};

mod keys;

/// **Private.** Per-project state directory holding `run.json` and `lock`.
const STATE_DIR: &str = ".kaanal";
/// **Private.** Port + token + pid of the running instance.
const RUN_FILE: &str = ".kaanal/run.json";
/// **Private.** OS-locked file enforcing one instance per directory (rule R6.7).
const LOCK_FILE: &str = ".kaanal/lock";

/// **Private.** Top-level CLI shape for clap.
#[derive(Parser)]
#[command(
    name = "kaanal",
    version,
    about = "A mirage of your backend: mock APIs, chaos and simulation"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// **Private.** Subcommands.
#[derive(Subcommand)]
enum Cmd {
    /// Start the emulator
    Up {
        #[arg(short, long, default_value = "kaanal.yaml")]
        config: PathBuf,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
    },
    /// Check config, topology and `OpenAPI` specs without starting
    Validate {
        #[arg(short, long, default_value = "kaanal.yaml")]
        config: PathBuf,
    },
    /// Print the JSON Schema for kaanal.yaml (point your editor's YAML plugin at it)
    Schema,
    /// Print skeleton endpoint entries for spec routes that have none (add-only)
    Endpoints {
        #[arg(short, long, default_value = "kaanal.yaml")]
        config: PathBuf,
        /// Append them to the endpoints file (creates kaanal.endpoints.yaml when kaanal.yaml has no endpoints:)
        #[arg(long)]
        write: bool,
    },
    /// Show nodes and active chaos of the running instance
    Status,
    /// Change chaos on the running instance
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },
}

/// **Private.** `chaos` subcommands.
#[derive(Subcommand)]
enum ChaosCmd {
    /// Set chaos on a node, e.g. `kaanal chaos set db --error-rate 100% --error 503`
    Set {
        node: String,
        /// Added latency, e.g. 300ms or 2s
        #[arg(long)]
        latency: Option<String>,
        /// Failure rate, e.g. 10% or 0.1
        #[arg(long = "error-rate")]
        error_rate: Option<String>,
        /// HTTP status for failures (500-504)
        #[arg(long)]
        error: Option<u16>,
        /// Take the node down (503 for every request)
        #[arg(long)]
        down: bool,
    },
    /// Clear chaos on one node, or on all nodes
    Clear { node: Option<String> },
}

/// **Private.** Exit code (RULES R6.3: 1 error, 2 not running) and message for stderr.
struct Fail(u8, String);

/// **Private.** Shorthand for an exit-1 failure.
fn err(msg: impl Into<String>) -> Fail {
    Fail(1, msg.into())
}

/// **Private.** Entry point: prints the failure message to stderr and returns the code.
fn main() -> ExitCode {
    // clap's own exit code for usage errors is 2, which here means "not running" (R6.3).
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(code, msg)) => {
            // Messages can quote config and spec contents; never let them drive the terminal.
            eprintln!("{}", clean_lines(&msg));
            ExitCode::from(code)
        }
    }
}

/// **Private.** Subcommand dispatch.
fn run(cli: Cli) -> Result<(), Fail> {
    match cli.cmd {
        Cmd::Validate { config } => {
            let (cfg, dir, src) = load(&config)?;
            let warnings =
                kaanal_server::check(&cfg, &dir).map_err(|e| located_err(&config, &src, &e))?;
            for w in warnings {
                eprintln!("warning: {}", clean(&w));
            }
            println!("ok: {}", config.display());
            Ok(())
        }
        Cmd::Schema => {
            let text = serde_json::to_string_pretty(&kaanal_core::config_schema())
                .map_err(|e| err(e.to_string()))?;
            println!("{text}");
            Ok(())
        }
        Cmd::Up { config, port, host } => up(&config, port, host),
        Cmd::Endpoints { config, write } => endpoints(&config, write),
        Cmd::Status => status(),
        Cmd::Chaos {
            cmd:
                ChaosCmd::Set {
                    node,
                    latency,
                    error_rate,
                    error,
                    down,
                },
        } => {
            let mut patch = Map::new();
            if let Some(l) = latency {
                patch.insert("latency_ms".into(), json!(parse_ms(&l).map_err(err)?));
            }
            if let Some(r) = error_rate {
                patch.insert("error_rate".into(), json!(parse_rate(&r).map_err(err)?));
            }
            if let Some(s) = error {
                patch.insert("error_status".into(), json!(s));
            }
            if down {
                patch.insert("down".into(), json!(true));
            }
            if patch.is_empty() {
                return Err(err(
                    "nothing to set: use --latency, --error-rate, --error or --down",
                ));
            }
            let path = format!("/__kaanal/chaos/{}", encode_segment(&node));
            let v = call("PUT", &path, Some(Value::Object(patch)))?;
            println!("{}", clean(&format!("{node}: {v}")));
            Ok(())
        }
        Cmd::Chaos {
            cmd: ChaosCmd::Clear { node },
        } => {
            let path = node.map_or("/__kaanal/chaos".to_string(), |n| {
                format!("/__kaanal/chaos/{}", encode_segment(&n))
            });
            call("DELETE", &path, None)?;
            println!("cleared");
            Ok(())
        }
    }
}

/// **Private.** `file:line:col: msg` when the error has a position, else `file: msg` (R6.1).
fn config_err(path: &Path, e: &ConfigError) -> Fail {
    // With a position, Display is `line:col: msg`, giving `file:line:col: msg`.
    let sep = if e.line.is_some() { ":" } else { ": " };
    err(format!("{}{sep}{e}", path.display()))
}

/// **Private.** A load error as `file:line:col: msg`: placed in the endpoints file when it
/// came from there (spec §8), else in `kaanal.yaml` (`src`).
fn located_err(config: &Path, src: &str, e: &Located) -> Fail {
    match &e.file {
        Some(f) => {
            let text = std::fs::read_to_string(f).unwrap_or_default();
            config_err(f, &e.in_source(&text))
        }
        None => config_err(config, &e.in_source(src)),
    }
}

/// **Private.** Reads and parses the config; errors print as `file:line:col: msg`
/// (R6.1). Also returns the config's directory, which spec files resolve against, and
/// the source, so later errors can be placed in it.
fn load(path: &Path) -> Result<(Config, PathBuf, String), Fail> {
    let src = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let cfg = Config::from_yaml(&src).map_err(|e| config_err(path, &e))?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    Ok((cfg, dir, src))
}

/// **Private.** How many users the config's auth node has, or `None` without an auth node.
fn auth_users(cfg: &Config) -> Option<usize> {
    cfg.nodes.values().find_map(|n| match n {
        NodeConfig::Auth(a) => Some(a.users.len()),
        _ => None,
    })
}

/// **Private.** The `up` lifecycle: private state dir → instance lock → signing key (auth
/// configs only) → token → tokio runtime → serve → run file → wait for Ctrl+C → clean up the
/// run file (only if it still holds our token).
fn up(config: &Path, port: Option<u16>, host: IpAddr) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    private_dir(Path::new(STATE_DIR))?;
    // Held until this process exits. The OS releases it even if we crash, so there is no stale lock,
    // and two `up` runs started at the same instant cannot both get past this point.
    let _lock = lock_instance()?;
    // X4: only an auth node needs a key. It is loaded (or created) before binding, so a bad
    // key file stops startup with its name; `validate` and `endpoints` never get here.
    let users = auth_users(&cfg);
    let auth_key = match users {
        Some(_) => Some(keys::signing_key(config)?),
        None => None,
    };
    if !host.is_loopback() {
        eprintln!(
            "warning: binding {host} makes the mock and control ports reachable from your network; \
             control calls still need the token and Host localhost/127.0.0.1/[::1]"
        );
    }
    let token = kaanal_server::new_token();
    let rt = tokio::runtime::Runtime::new().map_err(|e| err(e.to_string()))?;
    rt.block_on(async {
        let opts = ServeOpts {
            host,
            port: port.unwrap_or(cfg.port),
            token: token.clone(),
            auth_key,
        };
        let server = kaanal_server::serve(&cfg, &dir, opts)
            .await
            .map_err(|e| located_err(config, &src, &e))?;
        let port = server.addr.port();
        for w in &server.warnings {
            eprintln!("warning: {}", clean(w));
        }
        write_run_file(server.addr, &token)?;
        println!("Mocks: http://localhost:{port}");
        println!("UI:    http://localhost:{port}/__kaanal/ui#token={token}");
        if let Some(n) = users {
            println!(
                "Auth:  POST http://localhost:{port}/login, JWKS http://localhost:{port}/.well-known/jwks.json, {n} {}",
                if n == 1 { "user" } else { "users" }
            );
        }
        if let Some(seed) = server.drawn_seed {
            println!(
                "Seed:  {seed} (determinism: realistic; replay its timings and failures with determinism: controlled and seed: {seed}; generated data then follows that seed too)"
            );
        }
        // limitation: only Ctrl+C removes run.json. Closing the window, SIGTERM (docker stop) or a
        // crash leaves it behind; that is harmless (later commands exit 2, the next `up` overwrites it).
        // Add a SIGTERM handler under #[cfg(unix)] if containers become a use case.
        let res = tokio::select! {
            r = server.wait() => r.map_err(|e| err(e.to_string())),
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        // Only remove the file if it is still ours; a newer instance may have replaced it.
        if read_run().is_some_and(|(_, t)| t == token) {
            let _ = std::fs::remove_file(RUN_FILE);
        }
        res
    })
}

/// **Private.** First line of every block `kaanal endpoints` adds (D22).
const SKELETON_HEADER: &str = "# Added by kaanal endpoints";
/// **Private.** The file `kaanal endpoints --write` creates when `endpoints:` is absent.
const DEFAULT_ENDPOINTS_FILE: &str = "kaanal.endpoints.yaml";

/// **Private.** `kaanal endpoints [--write]` (E3, D22): skeleton entries for spec routes
/// missing from `endpoints:`. Add-only: it appends to an external file (refusing if the
/// result would not parse with every old key), creates `kaanal.endpoints.yaml` when there is
/// no `endpoints:` key, and only prints for an inline map (R1.17: kaanal.yaml is never rewritten).
fn endpoints(config: &Path, write: bool) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    let (topo, routes) =
        kaanal_server::spec_routes(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
    let (file, old, keys): (Option<PathBuf>, String, Vec<String>) = match &cfg.endpoints {
        Some(EndpointsConfig::Inline(m)) => (None, String::new(), m.keys().cloned().collect()),
        Some(EndpointsConfig::File(p)) => {
            let f = dir.join(p);
            let old = match std::fs::read_to_string(&f) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(err(format!("{}: {e}", f.display()))),
            };
            let keys = parse_endpoints(&old)
                .map_err(|e| config_err(&f, &e))?
                .into_keys()
                .collect();
            (Some(f), old, keys)
        }
        None => (None, String::new(), Vec::new()),
    };
    let lines = skeleton(&topo, &routes, &keys);
    if lines.is_empty() {
        println!("nothing to add: every spec route has an entry");
        return Ok(());
    }
    let block = |nl: &str| {
        std::iter::once(SKELETON_HEADER)
            .chain(lines.iter().map(String::as_str))
            .fold(String::new(), |mut s, l| {
                s.push_str(l);
                s.push_str(nl);
                s
            })
    };
    // Spec paths and node names are user text: never let them drive the terminal.
    // Under an inline map the lines are pasted below `endpoints:`, so they need its indent.
    let indent = if matches!(cfg.endpoints, Some(EndpointsConfig::Inline(_))) {
        "  "
    } else {
        ""
    };
    let show = || {
        for l in clean_lines(&block("\n")).lines() {
            println!("{indent}{l}");
        }
    };
    match (write, &cfg.endpoints, file) {
        (false, ..) => show(),
        (true, Some(EndpointsConfig::Inline(_)), _) => {
            show();
            eprintln!(
                "note: endpoints: is inline in {}; paste these lines under it (kaanal never rewrites it)",
                config.display()
            );
        }
        (true, _, Some(f)) => {
            let nl = if old.contains("\r\n") { "\r\n" } else { "\n" };
            let mut text = old.clone();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push_str(nl);
            }
            text.push_str(&block(nl));
            // Safety net (D22): the appended file must parse and keep every old entry.
            let back = parse_endpoints(&text).map_err(|e| {
                err(format!(
                    "{}: refusing to append, the result would not parse: {e} (a file that is one flow-style `{{ ... }}` map cannot be appended to: rewrite it in block style, or paste the lines in yourself)",
                    f.display()
                ))
            })?;
            if let Some(k) = keys.iter().find(|k| !back.contains_key(*k)) {
                return Err(err(format!(
                    "{}: refusing to append, entry '{k}' would be lost",
                    f.display()
                )));
            }
            std::fs::write(&f, text).map_err(|e| err(format!("{}: {e}", f.display())))?;
            println!("appended {} entries to {}", lines.len(), f.display());
        }
        (true, _, None) => {
            let f = dir.join(DEFAULT_ENDPOINTS_FILE);
            if f.exists() {
                return Err(err(format!(
                    "{} exists but {} has no endpoints: key; add `endpoints: {DEFAULT_ENDPOINTS_FILE}` to it, or move the file away",
                    f.display(),
                    config.display()
                )));
            }
            std::fs::write(&f, block("\n")).map_err(|e| err(format!("{}: {e}", f.display())))?;
            println!(
                "wrote {} entries to {}; add this line to {}: endpoints: {DEFAULT_ENDPOINTS_FILE}",
                lines.len(),
                f.display(),
                config.display()
            );
        }
    }
    Ok(())
}
/// **Private.** Creates `dir` (and missing parents) readable only by the current user, before
/// any secret is written into it (R5.8): `.kaanal`, and the signing-key directory (spec X16,
/// on every run, P16). Unix: mode 0700, or an error. Windows: the folder's ACL is replaced by
/// one grant for the current user (inheritance removed), so files created inside never exist
/// with a wider ACL; if that fails we warn and carry on: the token still protects the control API.
fn private_dir(dir: &Path) -> Result<(), Fail> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        b.mode(0o700);
        b.create(dir)
            .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)))
            .map_err(|e| err(format!("{}: {e}", dir.display())))?;
    }
    #[cfg(not(unix))]
    b.create(dir)
        .map_err(|e| err(format!("{}: {e}", dir.display())))?;
    #[cfg(windows)]
    if let Err(e) = restrict_to_current_user(dir) {
        eprintln!(
            "warning: could not restrict {} to the current user: {e}",
            dir.display()
        );
    }
    Ok(())
}

/// **Private.** Replaces the directory's ACL with one grant for the current user and removes
/// explicit grants to Everyone, Users and Authenticated Users, using System32 binaries by
/// full path (a planted earlier-PATH exe is never run).
#[cfg(windows)]
fn restrict_to_current_user(dir: &Path) -> Result<(), String> {
    use std::process::Command;
    let root = std::env::var("SystemRoot").map_err(|_| "SystemRoot is not set")?;
    // Full paths, so an icacls.exe or whoami.exe planted earlier on PATH is never run.
    let who = Command::new(format!(r"{root}\System32\whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|e| format!("whoami: {e}"))?;
    // Output looks like: "host\user","S-1-5-21-..."
    let text = String::from_utf8_lossy(&who.stdout);
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .map(|s| s.trim_matches('"'))
        .filter(|s| s.starts_with("S-1-"))
        .ok_or("could not read the current user's SID")?;
    let icacls = format!(r"{root}\System32\icacls.exe");
    let out = Command::new(&icacls)
        .arg(dir)
        .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(OI)(CI)F")])
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    // `/inheritance:r` drops inherited entries only. Remove explicit grants to the wide
    // well-known groups (Everyone, Users, Authenticated Users) too, on the folder and the
    // files already in it. Not `/reset`: it would briefly widen a running instance's files.
    let out = Command::new(&icacls)
        .arg(dir)
        .args([
            "/remove:g",
            "*S-1-1-0",
            "*S-1-5-32-545",
            "*S-1-5-11",
            "/T",
            "/C",
            "/Q",
        ])
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// **Private.** Exclusive advisory lock on `.kaanal/lock` (std `File::try_lock`, Rust 1.89+).
fn lock_instance() -> Result<std::fs::File, Fail> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(LOCK_FILE)
        .map_err(|e| err(format!("{LOCK_FILE}: {e}")))?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => {
            let port = read_run().map_or(String::new(), |(a, _)| format!(" on port {}", a.port()));
            Err(err(format!(
                "kaanal is already running{port} (this directory's {LOCK_FILE} is held)"
            )))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(err(format!("{LOCK_FILE}: {e}"))),
    }
}

/// **Private.** Writes `{host, port, token, pid}` via temp file + rename, so a concurrent
/// `status` never reads half a file. `host` is where the CLI connects: the bound address,
/// or loopback when bound to all interfaces.
fn write_run_file(bound: SocketAddr, token: &str) -> Result<(), Fail> {
    use std::io::Write;
    let host = match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let body =
        json!({ "host": host, "port": bound.port(), "token": token, "pid": std::process::id() })
            .to_string();
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    // Write a temp file, then rename over the real one, so a concurrent `kaanal status` never reads half a file.
    let tmp = format!("{RUN_FILE}.tmp");
    opts.open(&tmp)
        .and_then(|mut f| f.write_all(body.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, RUN_FILE))
        .map_err(|e| err(format!("{RUN_FILE}: {e}")))
}

/// **Private.** Reads `(address, token)` from the run file; any malformed piece → `None`.
/// A file without `host` (older versions) means `127.0.0.1`.
fn read_run() -> Option<(SocketAddr, String)> {
    let run: Value = serde_json::from_str(&std::fs::read_to_string(RUN_FILE).ok()?).ok()?;
    let port = u16::try_from(run["port"].as_u64()?).ok()?;
    let host = match run.get("host") {
        Some(h) => h.as_str()?.parse().ok()?,
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    Some((
        SocketAddr::new(host, port),
        run["token"].as_str()?.to_string(),
    ))
}

/// **Private.** Calls the running instance found via `.kaanal/run.json`. Exit 2 if
/// it isn't running. The client never follows redirects and never uses a proxy —
/// either could forward the token.
fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, Fail> {
    let (addr, token) =
        read_run().ok_or_else(|| Fail(2, format!("kaanal is not running (no {RUN_FILE})")))?;
    let port = addr.port();
    let auth = format!("Bearer {token}");
    let url = format!("http://{addr}{path}");
    // The control API only accepts a loopback Host with the bound port (R5.3), even when
    // we connect to a non-loopback bind address.
    let host = format!("localhost:{port}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // Never send the token through an HTTP(S)_PROXY from the environment.
        .proxy(None)
        // The server never redirects; following one could forward the token to another host.
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let res = match (method, body) {
        ("PUT", Some(b)) => agent
            .put(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_json(&b),
        ("DELETE", _) => agent
            .delete(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .call(),
        _ => agent
            .get(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .call(),
    };
    let mut resp = res.map_err(|e| Fail(2, format!("kaanal is not running on {addr}: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 300 {
        let msg = v["error"].as_str().unwrap_or(&text);
        return Err(err(clean(&format!(
            "{method} {path}: HTTP {status}: {msg}"
        ))));
    }
    Ok(v)
}

/// **Private.** Prints the node table with each node's live chaos settings.
fn status() -> Result<(), Fail> {
    let topo = call("GET", "/__kaanal/topology", None)?;
    let chaos = call("GET", "/__kaanal/chaos", None)?;
    println!("{:<12} {:<12} {:<10} CHAOS", "NODE", "TYPE", "FLAVOR");
    for n in topo["nodes"].as_array().into_iter().flatten() {
        let name = n["name"].as_str().unwrap_or("");
        let c = &chaos[name];
        let text = if c.is_null() {
            "-".to_string()
        } else {
            format!(
                "+{}ms, {:.0}% errors ({}){}",
                c["latency_ms"],
                c["error_rate"].as_f64().unwrap_or(0.0) * 100.0,
                c["error_status"],
                if c["down"] == true { ", DOWN" } else { "" }
            )
        };
        let flavor = n["flavor"].as_str().unwrap_or("-");
        let line = format!(
            "{:<12} {:<12} {:<10} {}",
            name,
            n["type"].as_str().unwrap_or(""),
            flavor,
            text
        );
        println!("{}", clean(&line));
    }
    Ok(())
}

/// **Private.** Server text may echo user input (e.g. a node name); never let it drive the terminal.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// **Private.** [`clean`] per line, keeping our own line breaks.
fn clean_lines(s: &str) -> String {
    s.lines().map(clean).collect::<Vec<_>>().join("\n")
}

/// **Private.** RFC 3986 unreserved characters; everything else in a path segment is escaped.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// **Private.** Percent-encodes one URL path segment.
fn encode_segment(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// **Private.** `300ms` / `2s` / `1m 30s` → milliseconds. A unit is required.
fn parse_ms(s: &str) -> Result<u64, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| format!("bad latency '{s}' (use e.g. 300ms or 2s)"))
}

/// **Private.** `10%` or `0.1` → a 0.0–1.0 rate.
fn parse_rate(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let v = match t.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().map(|p| p / 100.0),
        None => t.parse::<f64>(),
    }
    .map_err(|_| format!("bad error rate '{s}' (use e.g. 10% or 0.1)"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("error rate '{s}' must be between 0% and 100%"))
    }
}

#[cfg(test)]
mod tests {
    use super::{clean, encode_segment, parse_ms, parse_rate};

    #[test]
    fn latency_parsing() {
        assert_eq!(parse_ms("300ms"), Ok(300));
        assert_eq!(parse_ms("2s"), Ok(2000));
        assert_eq!(parse_ms("1m 30s"), Ok(90_000));
        assert!(parse_ms("15").is_err(), "a unit is required");
        assert_eq!(parse_ms(" 40 ms "), Ok(40));
        assert!(parse_ms("-5ms").is_err());
        assert!(parse_ms("fast").is_err());
        assert!(parse_ms("").is_err());
        assert_eq!(parse_ms("1.5s"), Ok(1500));
        assert!(parse_ms("99999999999999999999s").is_err());
    }

    #[test]
    fn rate_parsing() {
        assert_eq!(parse_rate("10%"), Ok(0.1));
        assert_eq!(parse_rate("100%"), Ok(1.0));
        assert_eq!(parse_rate("0.25"), Ok(0.25));
        assert_eq!(parse_rate("0"), Ok(0.0));
        assert!(parse_rate("150%").is_err());
        assert!(parse_rate("-1%").is_err());
        assert!(parse_rate("1.5").is_err());
        assert!(parse_rate("NaN").is_err());
        assert!(parse_rate("lots").is_err());
        assert!(parse_rate("").is_err());
    }

    #[test]
    fn clean_replaces_control_chars() {
        assert_eq!(clean("unknown node 'db'"), "unknown node 'db'");
        assert_eq!(clean("a\u{1b}[31mb\r\n\t\u{7f}\u{9b}"), "a?[31mb?????");
        assert_eq!(clean("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_segment("db-1_a.b~"), "db-1_a.b~");
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_segment("\u{1b}"), "%1B");
        assert_eq!(encode_segment("\u{e9}"), "%C3%A9");
    }
}
