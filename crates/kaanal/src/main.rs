//! The `kaanal` binary: CLI, process lifecycle, and a thin HTTP client for the
//! control API.
//!
//! `up`, `validate`, `schema` and `endpoints` work locally; `status` and `chaos` find the running
//! instance via `.kaanal/run.json` and call its control API (rule R2.6: every
//! interface is a thin client, no per-interface logic). `up` loads (or creates) the auth
//! signing key only when the config has an auth node (`keys`).

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use kaanal_core::chain::skeleton;
use kaanal_core::config::{NodeConfig, parse_endpoints};
use kaanal_core::diagnostics::Located;
use kaanal_core::{
    Config, ConfigError, EndpointsConfig, ScenarioConfig, parse_scenario, valid_name,
    validate_scenario_shape,
};
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
    /// Show nodes and the effective chaos (scenarios plus live) of the running instance
    Status,
    /// Switch named chaos scenarios on and off
    Scenario {
        #[command(subcommand)]
        cmd: ScenarioCmd,
    },
    /// Clean slate: deactivate scenarios, undefine runtime-defined ones (inline and file ones
    /// stay), clear live chaos, zero route counters, refill rate-limit buckets
    Reset,
    /// Change chaos on the running instance
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },
}

/// **Private.** `chaos` subcommands.
#[derive(Subcommand)]
enum ChaosCmd {
    /// Set chaos on a node, an endpoint, or a node only when reached through an endpoint,
    /// e.g. `kaanal chaos set db --error-rate 100% --error 503` or `--endpoint "GET /orders" --down`
    Set {
        /// The node (omit it to set chaos on the whole `--endpoint`)
        #[arg(required_unless_present = "endpoint")]
        node: Option<String>,
        /// Endpoint key, e.g. "GET /orders"; with a node: that node, only through this endpoint
        #[arg(long)]
        endpoint: Option<String>,
        /// How failures show up: status, reset, hang or malformed
        #[arg(long)]
        kind: Option<String>,
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
    /// Clear chaos on one node, one endpoint (`--endpoint`), or both together; none means everything
    Clear {
        node: Option<String>,
        /// Endpoint key, e.g. "GET /orders"
        #[arg(long)]
        endpoint: Option<String>,
    },
}

/// **Private.** `scenario` subcommands.
#[derive(Subcommand)]
enum ScenarioCmd {
    /// List every scenario with its source and whether it is active
    List,
    /// Show one scenario's chaos entries
    Show { name: String },
    /// Put a scenario on top of the stack
    Activate {
        name: String,
        /// Deactivate on its own after this long (100ms to 24h, e.g. 30s)
        #[arg(long)]
        ttl: Option<String>,
    },
    /// Take a scenario off the stack
    Deactivate { name: String },
    /// Set the whole stack, bottom to top (no names: deactivate all)
    Set { names: Vec<String> },
    /// Deactivate every scenario (live chaos stays)
    Clear,
    /// Print the active scenarios, bottom to top
    Active,
    /// Define a scenario from a YAML file (name = file stem) and activate it
    Load { file: PathBuf },
    /// Write scenarios/NAME.yaml from live chaos (default), the effective chaos or a scenario
    Save {
        name: String,
        /// live, effective or scenario:NAME
        #[arg(long, default_value = "live")]
        from: String,
        /// Replace an existing scenario or file
        #[arg(long)]
        force: bool,
        /// Clear the live chaos after saving
        #[arg(long = "clear-live")]
        clear_live: bool,
    },
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
        Cmd::Scenario { cmd } => scenario(cmd),
        Cmd::Reset => {
            call("POST", "/__kaanal/reset", None)?;
            println!("reset");
            Ok(())
        }
        Cmd::Chaos {
            cmd:
                ChaosCmd::Set {
                    node,
                    endpoint,
                    kind,
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
            if let Some(k) = kind {
                patch.insert("error_kind".into(), json!(k));
            }
            if down {
                patch.insert("down".into(), json!(true));
            }
            if patch.is_empty() {
                return Err(err(
                    "nothing to set: use --latency, --error-rate, --error, --kind or --down",
                ));
            }
            let label = match (&node, &endpoint) {
                (Some(n), Some(e)) => format!("{n} @ {e}"),
                (Some(n), None) => n.clone(),
                (None, e) => e.clone().unwrap_or_default(),
            };
            let v = if endpoint.is_some() {
                patch.extend(node.map(|n| ("node".to_string(), json!(n))));
                patch.extend(endpoint.map(|e| ("endpoint".to_string(), json!(e))));
                call("PUT", "/__kaanal/chaos", Some(Value::Object(patch)))?
            } else {
                // clap guarantees a node when there is no endpoint
                let path = format!(
                    "/__kaanal/chaos/{}",
                    encode_segment(&node.unwrap_or_default())
                );
                call("PUT", &path, Some(Value::Object(patch)))?
            };
            println!("{}", clean(&format!("{label}: {v}")));
            warn_all(&v);
            Ok(())
        }
        Cmd::Chaos {
            cmd: ChaosCmd::Clear { node, endpoint },
        } => {
            let path = match (node, endpoint) {
                (None, None) => "/__kaanal/chaos".to_string(),
                (Some(n), None) => format!("/__kaanal/chaos/{}", encode_segment(&n)),
                (n, Some(e)) => {
                    let node = n.map_or(String::new(), |n| format!("node={}&", encode_segment(&n)));
                    format!("/__kaanal/chaos?{node}endpoint={}", encode_segment(&e))
                }
            };
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
        Some(_) => {
            // A config that cannot load must not leave a key behind: check it first (the
            // same load `serve` does, so a topology or spec error shows before any keygen).
            kaanal_server::check(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
            Some(keys::signing_key(config)?)
        }
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

/// **Private.** Prints a response's `warnings` to stderr.
fn warn_all(v: &Value) {
    for w in v["warnings"].as_array().into_iter().flatten() {
        eprintln!("warning: {}", clean(w.as_str().unwrap_or("")));
    }
}

/// **Private.** `/__kaanal/scenarios/NAME` plus `tail`.
fn scenario_path(name: &str, tail: &str) -> String {
    format!("/__kaanal/scenarios/{}{tail}", encode_segment(name))
}

/// **Private.** The `active` names of a stack answer.
fn active_names(v: &Value) -> Vec<&str> {
    v["active"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// **Private.** The PUT body for a parsed scenario file.
fn scenario_body(cfg: &ScenarioConfig) -> Value {
    let mut body = json!({ "chaos": cfg.chaos });
    if let Some(d) = &cfg.description {
        body["description"] = json!(d);
    }
    body
}

/// **Private.** A scenario name checked here, so a reserved one (`active`, `save`, `reset`) is
/// refused with a message instead of hitting another route.
fn named(name: &str) -> Result<&str, Fail> {
    valid_name(name).map_err(err)?;
    Ok(name)
}

/// **Private.** `--from` as the API's tagged form: `live`, `effective` or `scenario:NAME`.
fn save_from(from: &str) -> Result<Value, Fail> {
    match from {
        "live" | "effective" => Ok(json!(from)),
        f => match f.strip_prefix("scenario:") {
            Some(n) if !n.is_empty() => Ok(json!({ "scenario": n })),
            _ => Err(err(format!(
                "bad --from '{}': use live, effective or scenario:NAME",
                clean(f)
            ))),
        },
    }
}

/// **Private.** `kaanal scenario ...`: thin calls to the control API, except `load`, which
/// parses the file here so a mistake shows as `file:line:col`.
fn scenario(cmd: ScenarioCmd) -> Result<(), Fail> {
    match cmd {
        ScenarioCmd::List => {
            let v = call("GET", "/__kaanal/scenarios", None)?;
            println!(
                "{:<24} {:<8} {:<7} DESCRIPTION",
                "NAME", "SOURCE", "ENTRIES"
            );
            for s in v.as_array().into_iter().flatten() {
                let line = format!(
                    "{:<24} {:<8} {:<7} {}{}",
                    s["name"].as_str().unwrap_or(""),
                    s["source"].as_str().unwrap_or(""),
                    s["entries"],
                    s["description"].as_str().unwrap_or(""),
                    active_suffix(s)
                );
                println!("{}", clean(&line));
            }
        }
        ScenarioCmd::Show { name } => {
            let v = call("GET", &scenario_path(named(&name)?, ""), None)?;
            let head = format!(
                "{} ({}){}",
                v["name"].as_str().unwrap_or(""),
                v["source"].as_str().unwrap_or(""),
                v["description"]
                    .as_str()
                    .map_or(String::new(), |d| format!(": {d}"))
            );
            println!("{}", clean(&head));
            for e in v["chaos"].as_array().into_iter().flatten() {
                println!("  - {}", clean(&e.to_string()));
            }
        }
        ScenarioCmd::Activate { name, ttl } => {
            let body = ttl_body(ttl)?;
            let v = call("POST", &scenario_path(named(&name)?, "/activate"), body)?;
            println!("active scenarios: {}", active_names(&v).join(", "));
        }
        ScenarioCmd::Deactivate { name } => {
            let name = named(&name)?;
            // ponytail: the answer is a bare 204, so "was it active" is read just before; a
            // concurrent change in between can make this line wrong, never the state.
            let was = call("GET", "/__kaanal/scenarios/active", None)?;
            call("POST", &scenario_path(name, "/deactivate"), None)?;
            if active_names(&was).contains(&name) {
                println!("deactivated {name}");
            } else {
                println!("ok: {name} is not active");
            }
        }
        ScenarioCmd::Set { names } => {
            let v = call("PUT", "/__kaanal/scenarios/active", Some(json!(names)))?;
            println!("active scenarios: {}", active_names(&v).join(", "));
        }
        ScenarioCmd::Clear => {
            call("DELETE", "/__kaanal/scenarios", None)?;
            println!("cleared");
        }
        ScenarioCmd::Active => {
            let v = call("GET", "/__kaanal/scenarios/active", None)?;
            for n in active_names(&v) {
                println!("{}", clean(n));
            }
        }
        ScenarioCmd::Load { file } => {
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| err(format!("{}: no usable file name", file.display())))?;
            valid_name(name).map_err(|m| err(format!("{}: {m}", file.display())))?;
            let src = std::fs::read_to_string(&file)
                .map_err(|e| err(format!("{}: {e}", file.display())))?;
            let cfg = parse_scenario(&src).map_err(|e| config_err(&file, &e))?;
            // Everything that needs no topology is checked here, placed in the file; the server
            // checks names against the topology.
            validate_scenario_shape(&cfg)
                .map_err(|e| config_err(&file, &e.located(Vec::new()).in_source(&src)))?;
            let body = scenario_body(&cfg);
            let v = call("PUT", &scenario_path(name, ""), Some(body))?;
            warn_all(&v);
            let v = call("POST", &scenario_path(name, "/activate"), None)?;
            println!(
                "loaded {name}; active scenarios: {}",
                active_names(&v).join(", ")
            );
        }
        ScenarioCmd::Save {
            name,
            from,
            force,
            clear_live,
        } => {
            let from = save_from(&from)?;
            let body =
                json!({ "name": name, "from": from, "force": force, "clear_live": clear_live });
            let v = call("POST", "/__kaanal/scenarios/save", Some(body))?;
            println!("saved {}", clean(v["path"].as_str().unwrap_or("")));
        }
    }
    Ok(())
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
        ("POST", Some(b)) => agent
            .post(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_json(&b),
        ("POST", None) => agent
            .post(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_empty(),
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

/// **Private.** Prints the node table with each node's effective chaos (scenarios plus live)
/// and the active scenarios.
fn status() -> Result<(), Fail> {
    let topo = call("GET", "/__kaanal/topology", None)?;
    // One call, so the stack and the chaos compiled from it come from the same snapshot.
    let active = call("GET", "/__kaanal/scenarios/active", None)?;
    let eff = &active["effective"];
    println!("{:<12} {:<12} {:<10} CHAOS", "NODE", "TYPE", "FLAVOR");
    for n in topo["nodes"].as_array().into_iter().flatten() {
        let name = n["name"].as_str().unwrap_or("");
        let c = &eff["nodes"][name];
        let text = if c.is_null() {
            "-".to_string()
        } else {
            chaos_text(c)
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
    // Endpoint-wide and node-at-endpoint chaos belong to no single row.
    for (key, c) in eff["endpoints"].as_object().into_iter().flatten() {
        println!("{}", clean(&format!("endpoint {key}: {}", chaos_text(c))));
    }
    for c in eff["node_at"].as_array().into_iter().flatten() {
        let (n, e) = (
            c["node"].as_str().unwrap_or(""),
            c["endpoint"].as_str().unwrap_or(""),
        );
        println!("{}", clean(&format!("{n} @ {e}: {}", chaos_text(c))));
    }
    let names = active_names(&active);
    println!(
        "active scenarios: {}",
        if names.is_empty() {
            "-".to_string()
        } else {
            clean(&names.join(", "))
        }
    );
    Ok(())
}

/// **Private.** One effective chaos entry as text: latency, each failure band (kinds other
/// than `status` show by name, `status` shows its code), then DOWN.
fn chaos_text(c: &Value) -> String {
    let mut s = format!("+{}ms", c["latency_ms"]);
    // `upto` is cumulative; show each band's own share of the requests.
    let mut prev = 0.0;
    for f in c["faults"].as_array().into_iter().flatten() {
        let upto = f["upto"].as_f64().unwrap_or(0.0);
        let own = upto - prev;
        prev = upto;
        let what = match f["kind"].as_str() {
            Some(k) if k != "status" => k.to_string(),
            _ => f["status"].to_string(),
        };
        let _ = write!(s, ", {:.0}% errors ({what})", own * 100.0);
    }
    if c["down"] == true {
        s.push_str(", DOWN");
    }
    s
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

/// **Private.** The activate body for `--ttl`, validated here so a typo sends nothing.
fn ttl_body(ttl: Option<String>) -> Result<Option<Value>, Fail> {
    ttl.map(|t| {
        let t = t.trim();
        humantime::parse_duration(t).map_err(|_| err(format!("bad ttl '{t}' (use e.g. 30s)")))?;
        Ok(json!({ "ttl": t }))
    })
    .transpose()
}

/// **Private.** ` (active)` or ` (active, expires in X)` for one list row, else nothing.
fn active_suffix(s: &Value) -> String {
    match (s["active"] == true, s["expires_in_ms"].as_u64()) {
        (true, Some(ms)) => format!(" (active, expires in {})", remaining(ms)),
        (true, None) => " (active)".to_string(),
        _ => String::new(),
    }
}

/// **Private.** Time left on a lease: whole seconds rounded up, or `Nms` under a second.
fn remaining(ms: u64) -> String {
    if ms >= 1000 {
        format!("{}s", ms.div_ceil(1000))
    } else {
        format!("{ms}ms")
    }
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
    use super::{clean, encode_segment, parse_ms, parse_rate, remaining, ttl_body};

    #[test]
    fn remaining_rounds_up_to_whole_seconds() {
        assert_eq!(remaining(999), "999ms");
        assert_eq!(remaining(1000), "1s");
        assert_eq!(remaining(1001), "2s");
        assert_eq!(remaining(59_001), "60s");
    }

    #[test]
    fn ttl_body_sends_the_trimmed_value() {
        let b = ttl_body(Some(" 30s ".into())).ok().flatten().unwrap();
        assert_eq!(b["ttl"], "30s");
        assert!(ttl_body(Some("soon".into())).is_err());
        assert!(matches!(ttl_body(None), Ok(None)));
    }

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
