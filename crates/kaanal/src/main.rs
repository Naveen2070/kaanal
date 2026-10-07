use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use kaanal_core::Config;
use kaanal_server::ServeOpts;
use serde_json::{Map, Value, json};

const STATE_DIR: &str = ".kaanal";
const RUN_FILE: &str = ".kaanal/run.json";
const LOCK_FILE: &str = ".kaanal/lock";

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
    /// Check config, topology and OpenAPI specs without starting
    Validate {
        #[arg(short, long, default_value = "kaanal.yaml")]
        config: PathBuf,
    },
    /// Show nodes and active chaos of the running instance
    Status,
    /// Change chaos on the running instance
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },
}

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

/// Exit code (RULES R6.3: 1 error, 2 not running) and message for stderr.
struct Fail(u8, String);

fn err(msg: impl Into<String>) -> Fail {
    Fail(1, msg.into())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(code, msg)) => {
            eprintln!("{msg}");
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<(), Fail> {
    match cli.cmd {
        Cmd::Validate { config } => {
            let (cfg, dir) = load(&config)?;
            let warnings = kaanal_server::check(&cfg, &dir)
                .map_err(|e| err(format!("{}: {e}", config.display())))?;
            for w in warnings {
                eprintln!("warning: {w}");
            }
            println!("ok: {}", config.display());
            Ok(())
        }
        Cmd::Up { config, port, host } => up(&config, port, host),
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

fn load(path: &Path) -> Result<(Config, PathBuf), Fail> {
    let src = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let cfg = Config::from_yaml(&src).map_err(|e| {
        // With a position, Display is `line:col: msg`, giving `file:line:col: msg` (R6.1).
        let sep = if e.line.is_some() { ":" } else { ": " };
        err(format!("{}{sep}{e}", path.display()))
    })?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    Ok((cfg, dir))
}

fn up(config: &Path, port: Option<u16>, host: IpAddr) -> Result<(), Fail> {
    let (cfg, dir) = load(config)?;
    private_dir()?;
    // Held until this process exits. The OS releases it even if we crash, so there is no stale lock,
    // and two `up` runs started at the same instant cannot both get past this point.
    let _lock = lock_instance()?;
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
        };
        let server = kaanal_server::serve(&cfg, &dir, opts)
            .await
            .map_err(|e| err(format!("{}: {e}", config.display())))?;
        let port = server.addr.port();
        for w in &server.warnings {
            eprintln!("warning: {w}");
        }
        write_run_file(port, &token)?;
        println!("Mocks: http://localhost:{port}");
        println!("UI:    http://localhost:{port}/__kaanal/ui#token={token}");
        // ponytail: limitation: only Ctrl+C removes run.json. Closing the window, SIGTERM (docker stop) or a
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

/// Creates `.kaanal` readable only by the current user, before any secret is written into it.
/// Unix: mode 0700. Windows: the folder's ACL is replaced by one grant for the current user
/// (inheritance removed), so files created inside never exist with a wider ACL.
/// If that fails we warn and carry on: the token still protects the control API.
fn private_dir() -> Result<(), Fail> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        b.mode(0o700);
        b.create(STATE_DIR)
            .and_then(|()| {
                std::fs::set_permissions(STATE_DIR, std::fs::Permissions::from_mode(0o700))
            })
            .map_err(|e| err(format!("{STATE_DIR}: {e}")))?;
    }
    #[cfg(not(unix))]
    b.create(STATE_DIR)
        .map_err(|e| err(format!("{STATE_DIR}: {e}")))?;
    #[cfg(windows)]
    if let Err(e) = restrict_to_current_user(STATE_DIR) {
        eprintln!("warning: could not restrict {STATE_DIR} to the current user: {e}");
    }
    Ok(())
}

#[cfg(windows)]
fn restrict_to_current_user(dir: &str) -> Result<(), String> {
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
    let out = Command::new(format!(r"{root}\System32\icacls.exe"))
        .args([
            dir,
            "/inheritance:r",
            "/grant:r",
            &format!("*{sid}:(OI)(CI)F"),
        ])
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// Exclusive advisory lock on `.kaanal/lock` (std `File::try_lock`, Rust 1.89+).
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
            let port = read_run().map_or(String::new(), |(p, _)| format!(" on port {p}"));
            Err(err(format!(
                "kaanal is already running{port} (this directory's {LOCK_FILE} is held)"
            )))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(err(format!("{LOCK_FILE}: {e}"))),
    }
}

fn write_run_file(port: u16, token: &str) -> Result<(), Fail> {
    use std::io::Write;
    let body = json!({ "port": port, "token": token, "pid": std::process::id() }).to_string();
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

fn read_run() -> Option<(u16, String)> {
    let run: Value = serde_json::from_str(&std::fs::read_to_string(RUN_FILE).ok()?).ok()?;
    let port = u16::try_from(run["port"].as_u64()?).ok()?;
    Some((port, run["token"].as_str()?.to_string()))
}

/// Calls the running instance found via `.kaanal/run.json`. Exit 2 if it isn't running.
fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, Fail> {
    let (port, token) =
        read_run().ok_or_else(|| Fail(2, format!("kaanal is not running (no {RUN_FILE})")))?;
    let auth = format!("Bearer {token}");
    // The control API only accepts a loopback Host with the bound port (R5.3).
    let url = format!("http://127.0.0.1:{port}{path}");
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
        ("PUT", Some(b)) => agent.put(&url).header("Authorization", &auth).send_json(&b),
        ("DELETE", _) => agent.delete(&url).header("Authorization", &auth).call(),
        _ => agent.get(&url).header("Authorization", &auth).call(),
    };
    let mut resp =
        res.map_err(|e| Fail(2, format!("kaanal is not running on port {port}: {e}")))?;
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

fn status() -> Result<(), Fail> {
    let topo = call("GET", "/__kaanal/topology", None)?;
    let chaos = call("GET", "/__kaanal/chaos", None)?;
    println!("{:<12} {:<10} {:<10} CHAOS", "NODE", "TYPE", "FLAVOR");
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
            "{:<12} {:<10} {:<10} {}",
            name,
            n["type"].as_str().unwrap_or(""),
            flavor,
            text
        );
        println!("{}", clean(&line));
    }
    Ok(())
}

/// Server text may echo user input (e.g. a node name); never let it drive the terminal.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Percent-encodes one URL path segment (RFC 3986 unreserved characters stay as they are).
fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn parse_ms(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (num, mult) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1000)
    } else {
        (t, 1)
    };
    num.trim()
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
        .ok_or_else(|| format!("bad latency '{s}' (use e.g. 300ms or 2s)"))
}

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
        assert_eq!(parse_ms("15"), Ok(15));
        assert_eq!(parse_ms(" 40 ms "), Ok(40));
        assert!(parse_ms("-5ms").is_err());
        assert!(parse_ms("fast").is_err());
        assert!(parse_ms("").is_err());
        assert!(parse_ms("1.5s").is_err());
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
