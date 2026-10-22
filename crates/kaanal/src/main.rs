//! The `kaanal` binary: CLI, process lifecycle, and a thin HTTP client for the
//! control API.
//!
//! `up`, `validate`, `schema` and `endpoints` work locally; `status` and `chaos` find the running
//! instance via `.kaanal/run.json` and call its control API (rule R2.6: every
//! interface is a thin client, no per-interface logic). `up` loads (or creates) the auth
//! signing key only when the config has an auth node (`keys`).
//!
//! Exit codes (RULES R6.3, amended by R1.45): 0 ok, 1 error, 2 not running, 3 reserved, 64 usage
//! error (clap's own errors and every bad argument found before a request; `--help` is 0).

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use clap::{Args, CommandFactory, Parser, Subcommand};
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
mod lifecycle;
mod logview;
mod output;
mod target;

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
    /// Print read commands' results as JSON (also `KAANAL_JSON=1`); errors stay text on stderr
    #[arg(long, global = true)]
    json: bool,
    /// Talk to the instance at http://HOST:PORT instead of the one in .kaanal/ (also
    /// `KAANAL_ENDPOINT`); goes before the subcommand, whose own --endpoint is a route key
    #[arg(long, value_name = "URL")]
    endpoint: Option<String>,
    /// Token for --endpoint. Prefer `KAANAL_TOKEN`: a value given here is visible in process
    /// lists and shell history
    #[arg(long, global = true, value_name = "TOKEN")]
    token: Option<String>,
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
        /// Start in the background, print the URLs once it answers, and return
        #[arg(short, long, conflicts_with = "detached")]
        detach: bool,
        /// With -d: how long to wait for it to answer, 1ms to 1h
        #[arg(long, default_value = "15s", value_parser = start_budget, requires = "detach")]
        wait: Duration,
        /// Run as a background child of `up -d`: keep the token out of the output, ignore hangup
        #[arg(long, hide = true)]
        detached: bool,
    },
    /// Print a shell completion script (bash, zsh, fish, powershell, elvish) to stdout
    Completions {
        /// The shell to generate the script for
        shell: clap_complete::Shell,
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
    /// Stop the running instance: drain, then force-kill (a verified kaanal process only) if it won't stop
    Down {
        /// How long in-flight requests may take to finish, 1ms to 300s
        #[arg(long, default_value = "5s", value_parser = drain_budget)]
        timeout: Duration,
    },
    /// Show nodes and the effective chaos (scenarios plus live) of the running instance
    Status,
    /// Switch named chaos scenarios on and off
    Scenario {
        #[command(subcommand)]
        cmd: ScenarioCmd,
    },
    /// Step an endpoint's live pin to the next UI state (loading, empty, error, slow, success),
    /// or pin one with --state; e.g. `kaanal cycle "GET /orders"`
    Cycle {
        /// Endpoint key, e.g. "GET /orders"
        endpoint: String,
        /// loading, empty, error, slow or success
        #[arg(long)]
        state: Option<String>,
        /// Response status (400-599 for error, 200-299 for empty)
        #[arg(long)]
        status: Option<u16>,
        /// Extra delay before the answer, e.g. 2s
        #[arg(long)]
        delay: Option<String>,
        /// Remove this endpoint's live pin
        #[arg(long, conflicts_with_all = ["state", "status", "delay"])]
        off: bool,
    },
    /// Show the live pins and the effective pin of each endpoint with its source
    Pins,
    /// Clean slate: deactivate scenarios, undefine runtime-defined ones (inline and file ones
    /// stay), clear live chaos, zero route counters, refill rate-limit buckets
    Reset,
    /// Show the last finished requests, one line each (`-f` to follow live)
    Logs(LogsArgs),
    /// Show one request as a per-hop waterfall: an id like r04f2-0042, `last`, or the client's X-Request-Id
    Trace { key: String },
    /// Change chaos on the running instance
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },
    /// Take a node down or bring it back
    Node {
        #[command(subcommand)]
        cmd: NodeCmd,
    },
}

/// **Private.** `node` subcommands.
#[derive(Subcommand)]
enum NodeCmd {
    /// Take a node down (503 for every request), e.g. `kaanal node down db --for 30s`
    Down {
        node: String,
        /// Bring it back after this long (100ms to 24h, e.g. 30s)
        #[arg(long = "for", value_name = "DURATION")]
        for_: Option<String>,
    },
    /// List nodes with their up/down state and chaos
    List,
    /// Clear a node's down flag (other chaos stays); fails if a scenario or override still holds it down
    Up { node: String },
}

/// **Private.** Flags of `kaanal logs`.
#[derive(Args)]
struct LogsArgs {
    /// How many of the latest requests to show (1-1000)
    #[arg(short = 'n', default_value_t = 50)]
    n: u32,
    /// Keep printing new requests as they finish (Ctrl+C to stop)
    #[arg(short, long)]
    follow: bool,
    /// Only requests newer than this, e.g. 5m or an RFC 3339 time
    #[arg(long)]
    since: Option<String>,
    /// Route template as logged, e.g. "GET /users/{id}"
    #[arg(long)]
    endpoint: Option<String>,
    /// Statuses, e.g. 404 or 404,5xx
    #[arg(long)]
    status: Option<String>,
    /// Only requests a node failed
    #[arg(long)]
    failed: bool,
    /// Only requests that ran under this scenario scope
    #[arg(long)]
    scope: Option<String>,
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
        /// Revert what this sets after this long (100ms to 24h, e.g. 30s)
        #[arg(long = "for", value_name = "DURATION")]
        for_: Option<String>,
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

/// **Private.** Exit code (RULES R6.3 as amended by R1.45: 0 ok, 1 error, 2 not running, 3
/// reserved, 64 usage error) and message for stderr.
#[derive(Debug)]
struct Fail(u8, String);

impl Fail {
    /// **Private.** A bad argument, found before any request is made: exit 64 (spec M13). A value
    /// the server refuses (HTTP 400) is a runtime error and stays 1.
    fn usage(msg: impl Into<String>) -> Fail {
        Fail(64, msg.into())
    }
}

/// **Private.** Shorthand for an exit-1 failure.
fn err(msg: impl Into<String>) -> Fail {
    Fail(1, msg.into())
}

/// **Private.** Entry point: prints the failure message to stderr and returns the code.
fn main() -> ExitCode {
    // clap's own exit code for usage errors is 2, which here means "not running" (R6.3); they are 64.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(64)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    output::set_json(cli.json || output::env_on());
    target::set_overrides(cli.endpoint.clone(), cli.token.clone());
    match target::check_flags().and_then(|()| run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(code, msg)) => {
            // Messages can quote config and spec contents; never let them drive the terminal.
            eprintln!("{}", clean_lines(&msg));
            ExitCode::from(code)
        }
    }
}

/// **Private.** Usage error (64) `why, but SOURCE is set` when an endpoint comes from the flag or env.
fn no_endpoint(why: &str) -> Result<(), Fail> {
    match target::endpoint_source() {
        Some(src) => Err(Fail::usage(format!("{why}, but {src} is set"))),
        None => Ok(()),
    }
}

/// **Private.** `kaanal schema`: the JSON Schema of `kaanal.yaml`.
fn schema() -> Result<(), Fail> {
    let text = serde_json::to_string_pretty(&kaanal_core::config_schema())
        .map_err(|e| err(e.to_string()))?;
    out_bytes(
        format!(
            "{text}
"
        )
        .as_bytes(),
    );
    Ok(())
}

/// **Private.** `kaanal completions SHELL`: the script on stdout; no instance, ignores `--json`.
/// Generated in memory: `clap_complete` panics on a closed pipe, `out_bytes` does not.
fn completions(shell: clap_complete::Shell) {
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "kaanal", &mut buf);
    out_bytes(&buf);
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
        Cmd::Completions { shell } => {
            completions(shell);
            Ok(())
        }
        Cmd::Schema => schema(),
        Cmd::Up {
            config,
            port,
            host,
            detach,
            wait,
            detached,
        } => {
            // A process-bound command: an endpoint (even an exported one) is refused loudly (E6).
            no_endpoint("up starts a local instance and takes no endpoint")?;
            if detach {
                up_detached(&config, port, host, wait)
            } else {
                up(&config, port, host, detached)
            }
        }
        Cmd::Endpoints { config, write } => endpoints(&config, write),
        Cmd::Down { timeout } => {
            if target::endpoint_source().is_some() {
                down_remote(&target::resolve()?, timeout)
            } else {
                down(timeout)
            }
        }
        Cmd::Status => status(),
        Cmd::Scenario { cmd } => scenario(cmd),
        Cmd::Cycle {
            endpoint,
            state,
            status,
            delay,
            off,
        } => cycle(&endpoint, state, status, delay, off),
        Cmd::Pins => pins(),
        Cmd::Logs(a) => logs_cmd(&a),
        Cmd::Trace { key } => trace(&key),
        Cmd::Reset => {
            let v = call("POST", "/__kaanal/reset", None)?;
            output::emit(&v, || println!("reset"));
            Ok(())
        }
        Cmd::Chaos { cmd } => chaos(cmd),
        Cmd::Node { cmd } => node(cmd),
    }
}

/// **Private.** `kaanal chaos set|clear`.
fn chaos(cmd: ChaosCmd) -> Result<(), Fail> {
    match cmd {
        ChaosCmd::Set {
            node,
            endpoint,
            kind,
            latency,
            error_rate,
            error,
            down,
            for_,
        } => {
            let mut patch = chaos_patch(latency, error_rate, error, kind, down)?;
            let revert = for_set(&mut patch, for_)?;
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
            output::emit(&v, || {
                println!("{}", clean(&format!("{label}: {v}{revert}")));
            });
            warn_all(&v);
            Ok(())
        }
        ChaosCmd::Clear { node, endpoint } => {
            let path = chaos_clear_path(node, endpoint);
            let v = call("DELETE", &path, None)?;
            output::emit(&v, || println!("cleared"));
            Ok(())
        }
    }
}

/// **Private.** The body of `chaos set`, checked here so a typo sends nothing.
fn chaos_patch(
    latency: Option<String>,
    error_rate: Option<String>,
    error: Option<u16>,
    kind: Option<String>,
    down: bool,
) -> Result<Map<String, Value>, Fail> {
    let mut patch = Map::new();
    if let Some(l) = latency {
        patch.insert(
            "latency_ms".into(),
            json!(parse_ms(&l).map_err(Fail::usage)?),
        );
    }
    if let Some(r) = error_rate {
        patch.insert(
            "error_rate".into(),
            json!(parse_rate(&r).map_err(Fail::usage)?),
        );
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
        return Err(Fail::usage(
            "nothing to set: use --latency, --error-rate, --error, --kind or --down",
        ));
    }
    Ok(patch)
}

/// **Private.** Adds a validated `--for` to the patch as `ttl`; returns ` (reverts in D)` for the
/// text output, or nothing.
fn for_set(patch: &mut Map<String, Value>, d: Option<String>) -> Result<String, Fail> {
    let Some(Value::Object(b)) = ttl_body("--for", d)? else {
        return Ok(String::new());
    };
    let text = format!(" (reverts in {})", b["ttl"].as_str().unwrap_or_default());
    patch.extend(b);
    Ok(text)
}

/// **Private.** `kaanal node down|up`: `PUT /chaos/NODE` with only the down flag.
fn node(cmd: NodeCmd) -> Result<(), Fail> {
    match cmd {
        NodeCmd::Down { node, for_ } => {
            let mut patch = Map::from_iter([("down".to_string(), json!(true))]);
            let revert = for_set(&mut patch, for_)?;
            let v = call("PUT", &node_path(&node), Some(Value::Object(patch)))?;
            output::emit(&v, || println!("{}: down{revert}", clean(&node)));
        }
        NodeCmd::List => {
            let topo = call("GET", "/__kaanal/topology", None)?;
            let active = call("GET", "/__kaanal/scenarios/active", None)?;
            let live = call("GET", "/__kaanal/chaos", None)?;
            let rows = output::node_rows(&topo, &active, &live);
            output::emit(&Value::Array(rows.clone()), || {
                output::print_node_table(&rows);
            });
        }
        NodeCmd::Up { node } => {
            let v = call("PUT", &node_path(&node), Some(json!({ "down": false })))?;
            // The server leaves calm keys out of `effective`, so a missing entry means "not down";
            // a present entry always carries a bool `down` (resolved_json).
            if v["effective"]["nodes"][&node]["down"] == true {
                return Err(err(format!(
                    "{node} is still down: held by an active scenario or override (see kaanal scenario active)"
                )));
            }
            output::emit(&v, || println!("{}: up", clean(&node)));
        }
    }
    Ok(())
}

/// **Private.** `/__kaanal/chaos/NODE`.
fn node_path(node: &str) -> String {
    format!("/__kaanal/chaos/{}", encode_segment(node))
}

/// **Private.** The `DELETE` path of `chaos clear` for a node, an endpoint or both.
fn chaos_clear_path(node: Option<String>, endpoint: Option<String>) -> String {
    match (node, endpoint) {
        (None, None) => "/__kaanal/chaos".to_string(),
        (Some(n), None) => format!("/__kaanal/chaos/{}", encode_segment(&n)),
        (n, Some(e)) => {
            let node = n.map_or(String::new(), |n| format!("node={}&", encode_segment(&n)));
            format!("/__kaanal/chaos?{node}endpoint={}", encode_segment(&e))
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
/// configs only) → token → tokio runtime → serve → run file → wait for a stop signal or
/// `POST /__kaanal/shutdown` → drain → clean up the run file (only if it still holds our token).
/// `detached` (the hidden `--detached`) is `up -d`'s child: its stdout is a log file, so the UI
/// URL carries no token (M4), and it ignores SIGHUP (M11).
fn up(config: &Path, port: Option<u16>, host: IpAddr, detached: bool) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    private_dir(Path::new(STATE_DIR))?;
    // Held until this process exits. The OS releases it even if we crash, so there is no stale lock,
    // and two `up` runs started at the same instant cannot both get past this point.
    let _lock = lock_instance()?;
    // The lock proves no live instance owns run.json, so whatever is there is a dead one's (M9).
    // Removed now, not overwritten after binding, so nothing reads it as live during startup.
    let _ = std::fs::remove_file(RUN_FILE);
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
    exposure_warning(host);
    let token = kaanal_server::new_token();
    let rt = tokio::runtime::Runtime::new().map_err(|e| err(e.to_string()))?;
    rt.block_on(async {
        // Handlers first, before anything is published: a signal that arrives while we start up
        // is queued and drained, not fatal (and `up -d`'s child survives a hangup from the start).
        let stop_requested = lifecycle::stop_signal();
        if detached {
            lifecycle::ignore_hangup();
        }
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
        let auth = users.map(|n| {
            format!(
                "POST http://localhost:{port}/login, JWKS http://localhost:{port}/.well-known/jwks.json, {n} {}",
                if n == 1 { "user" } else { "users" }
            )
        });
        write_run_file(
            server.addr,
            &token,
            &json!({ "auth": auth, "seed": server.drawn_seed, "warnings": server.warnings }),
        )?;
        println!("Mocks: http://localhost:{port}");
        if detached {
            println!("UI:    http://localhost:{port}/__kaanal/ui");
        } else {
            println!("UI:    http://localhost:{port}/__kaanal/ui#token={token}");
        }
        if let Some(a) = &auth {
            println!("Auth:  {a}");
        }
        if let Some(seed) = server.drawn_seed {
            println!("{}", seed_line(seed));
        }
        // A stop signal (or POST /__kaanal/shutdown, which ends `wait` by itself) drains for up
        // to 5s, then cuts what is left; either way the run file goes below.
        let stop = server.stopper();
        let wait = server.wait();
        tokio::pin!(wait);
        let res = tokio::select! {
            r = &mut wait => r,
            () = stop_requested => {
                stop(Duration::from_secs(5));
                wait.await
            }
        };
        // Only remove the file if it is still ours; a newer instance may have replaced it.
        if lifecycle::read_run(Path::new(STATE_DIR)).is_some_and(|r| r.token == token) {
            let _ = std::fs::remove_file(RUN_FILE);
        }
        res.map_err(|e| err(e.to_string()))
    })
}

/// **Private.** The network-exposure warning for a non-loopback `--host`.
fn exposure_text(host: IpAddr) -> String {
    format!(
        "warning: binding {host} makes the mock and control ports reachable from your network; control calls still need the token and Host localhost/127.0.0.1/[::1]"
    )
}

/// **Private.** Prints [`exposure_text`] to stderr for a non-loopback `--host`.
fn exposure_warning(host: IpAddr) {
    if !host.is_loopback() {
        eprintln!("{}", exposure_text(host));
    }
}

/// **Private.** The `Seed:` line `up` prints under `determinism: realistic`.
fn seed_line(seed: u64) -> String {
    format!(
        "Seed:  {seed} (determinism: realistic; replay its timings and failures with determinism: controlled and seed: {seed}; generated data then follows that seed too)"
    )
}

/// **Private.** `--wait` of `up -d`. A bad value is a clap error, which `main` turns into exit 64.
fn start_budget(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .filter(|d| (Duration::from_millis(1)..=Duration::from_secs(3600)).contains(d))
        .ok_or_else(|| format!("bad wait '{s}' (1ms to 1h, e.g. 15s)"))
}

/// **Private.** An HTTP agent for the control API: no proxy (it could forward the token), no
/// redirects, short timeouts.
fn control_agent(total: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_global(Some(total))
        .build()
        .into()
}

/// **Private.** The answer to `POST /__kaanal/shutdown`.
struct ShutdownReply {
    status: u16,
    /// The drain time left, from the 202 body (a drain that was already running reports its own).
    left: Option<Duration>,
}

/// **Private.** `POST /__kaanal/shutdown` with a drain budget such as `5s`: the answer, or `None`
/// when there was no answer at all.
fn post_shutdown(run: &lifecycle::Run, budget: &str) -> Option<ShutdownReply> {
    let mut r = control_agent(Duration::from_secs(5))
        .post(&format!("http://{}/__kaanal/shutdown", run.addr))
        .header("Host", &format!("localhost:{}", run.addr.port()))
        .header("Authorization", &format!("Bearer {}", run.token))
        .send_json(json!({ "timeout": budget }))
        .ok()?;
    let status = r.status().as_u16();
    let left = r
        .body_mut()
        .read_json::<Value>()
        .ok()
        .and_then(|v| v["timeout_ms"].as_u64())
        .map(|ms| lifecycle::server_wait(Some(ms), 0));
    Some(ShutdownReply { status, left })
}

/// **Private.** How long `down` waits for the lock after asking: its own budget, or the time left
/// of an earlier, longer drain if that is more (a second `down` must not cut it short), plus 2 s.
fn drain_wait(timeout: Duration, reply: Option<&ShutdownReply>) -> Duration {
    timeout.max(reply.and_then(|r| r.left).unwrap_or_default()) + Duration::from_secs(2)
}

/// **Private.** The lines `up` prints once the server is up, rebuilt from `run.json`'s `startup`.
/// The UI URL carries the token: the caller's terminal is the one place it may go.
fn print_started(run: &lifecycle::Run) {
    let port = run.addr.port();
    println!("Mocks: http://localhost:{port}");
    println!(
        "UI:    http://localhost:{port}/__kaanal/ui#token={}",
        run.token
    );
    if let Some(a) = &run.startup.auth {
        println!("Auth:  {}", clean(a));
    }
    if let Some(seed) = run.startup.seed {
        println!("{}", seed_line(seed));
    }
    for w in &run.startup.warnings {
        eprintln!("warning: {}", clean(w));
    }
}

/// **Private.** An instance that is already up: print its lines and `already running (pid N)`.
fn print_running(run: &lifecycle::Run) {
    print_started(run);
    println!(
        "already running (pid {})",
        run.pid.map_or("unknown".to_string(), |p| p.to_string())
    );
}

/// **Private.** Looks at the directory once: free (stale state cleaned), a healthy instance, or
/// busy (the lock is held but nothing answers yet: starting, or draining).
fn look(state: &Path) -> Result<lifecycle::Look, Fail> {
    if lifecycle::clear_stale(state)?.is_some() {
        return Ok(lifecycle::Look::Free);
    }
    Ok(lifecycle::read_run(state)
        .filter(|r| lifecycle::healthy(r.addr))
        .map_or(lifecycle::Look::Busy, lifecycle::Look::Healthy))
}

/// **Private.** `kaanal up -d` (spec M3): validate here, then start `up --detached` in the
/// background with its output in `.kaanal/kaanal.log`, wait until it answers, print its lines.
fn up_detached(config: &Path, port: Option<u16>, host: IpAddr, wait: Duration) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    // The same checks `serve` makes, before anything is spawned or created: a bad config is
    // an error here, with its file:line:col, not a log tail of a child that died.
    kaanal_server::check(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
    let state = Path::new(STATE_DIR);
    if !state.exists() {
        private_dir(state)?;
    }
    // One `up -d` at a time per directory, from here until the child answers: a second one blocks
    // (for up to the winner's --wait plus its checks, not its own --wait; the two can add up),
    // then finds the instance running and prints it (exit 0) instead of rotating the live log away.
    // The child's instance lock stays the final arbiter. Not inherited by the child.
    let _start = lock_start(state)?;
    // A healthy instance is the answer; one that is starting or draining is waited for (within
    // --wait); a free directory (stale state cleaned) means start fresh.
    if let lifecycle::Existing::Running(run) =
        lifecycle::existing_instance(wait, Duration::from_millis(100), || look(state))?
    {
        print_running(&run);
        return Ok(());
    }
    exposure_warning(host);
    let log_path = lifecycle::rotate_log(state)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .map_err(|e| err(format!("{}: {e}", log_path.display())))?;
    let exe =
        std::env::current_exe().map_err(|e| err(format!("cannot find kaanal itself: {e}")))?;
    let cwd = std::env::current_dir().map_err(|e| err(e.to_string()))?;
    let args: Vec<std::ffi::OsString> = vec![
        "up".into(),
        "-c".into(),
        config.into(),
        "--port".into(),
        port.unwrap_or(cfg.port).to_string().into(),
        "--host".into(),
        host.to_string().into(),
        "--detached".into(),
    ];
    let mut child = lifecycle::spawn_detached(&exe, &args, log, &cwd)?;
    let log_shown = std::path::absolute(&log_path).unwrap_or(log_path.clone());
    let began = std::time::Instant::now();
    let ready = lifecycle::wait_ready(state, &mut child, wait, Duration::from_millis(100));
    let why = match ready {
        Ok(run) => {
            print_started(&run);
            println!("Log: {}", clean(&log_shown.display().to_string()));
            println!("Stop: kaanal down");
            return Ok(());
        }
        Err(lifecycle::ReadyError::ChildExited) => {
            // Lost a race with a simultaneous `up -d`: the instance that won is the answer.
            let left = wait.saturating_sub(began.elapsed());
            if let Some(run) =
                lifecycle::after_child_exit(left, Duration::from_millis(100), || look(state))?
            {
                print_running(&run);
                return Ok(());
            }
            "the background process exited".to_string()
        }
        Err(lifecycle::ReadyError::TimedOut) => {
            let pid = child.id();
            let healthy = || {
                lifecycle::run_of(state, pid)
                    .filter(|r| lifecycle::healthy(r.addr))
                    .filter(|_| {
                        matches!(
                            lifecycle::lock_probe(state),
                            Ok(lifecycle::Liveness::Running)
                        )
                    })
            };
            let settled = lifecycle::settle_unready(
                healthy,
                || lifecycle::run_of(state, pid),
                |run| {
                    post_shutdown(run, "5s");
                },
                || {
                    lifecycle::wait_lock_free(state, Duration::from_secs(7));
                },
                || {
                    let _ = child.kill();
                },
            );
            if let lifecycle::Settled::Healthy(run) = settled {
                eprintln!(
                    "note: kaanal answered just after the {} wait; leaving it running",
                    humantime::format_duration(wait)
                );
                print_started(&run);
                println!("Log: {}", clean(&log_shown.display().to_string()));
                println!("Stop: kaanal down");
                return Ok(());
            }
            let _ = child.wait();
            format!("no answer within {}", humantime::format_duration(wait))
        }
    };
    Err(err(format!(
        "kaanal did not start: {why}; last lines of {}:\n{}",
        log_shown.display(),
        Some(lifecycle::tail_log(&log_path, 20))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "(log is empty)".to_string())
    )))
}

/// **Private.** Takes `.kaanal/start.lock`, waiting for any other `up -d` in this directory.
fn lock_start(state: &Path) -> Result<std::fs::File, Fail> {
    let path = state.join("start.lock");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    f.lock()
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    Ok(f)
}

/// **Private.** `--timeout` of `down`: a duration the server's shutdown endpoint accepts. A bad
/// value is a clap error, which `main` turns into exit 64.
fn drain_budget(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .filter(|d| (Duration::from_millis(1)..=Duration::from_secs(300)).contains(d))
        .ok_or_else(|| format!("bad timeout '{s}' (1ms to 300s, e.g. 5s)"))
}

/// **Private.** `kaanal down` (spec M7): ask the instance to drain, wait for its lock to go, and
/// only if that fails kill it, and only after checking the pid really is a kaanal process.
fn down(timeout: Duration) -> Result<(), Fail> {
    let state = Path::new(STATE_DIR);
    if let Some(msg) = lifecycle::clear_stale(state)? {
        println!("{}", clean(&msg));
        return Ok(());
    }
    // Lock held: an instance, possibly still starting. Never touch its files; if it vanishes
    // meanwhile, that is a stop, not a failure.
    let run = match lifecycle::run_or_gone(state, lifecycle::STARTUP_WAIT)? {
        lifecycle::Found::Gone(msg) => {
            println!("{}", clean(&msg));
            return Ok(());
        }
        lifecycle::Found::Run(r) => r,
    };
    let budget = humantime::format_duration(timeout).to_string();
    // 202, or no answer at all (refused, or cut by a tiny budget before the 202 left): the drain
    // is running or the process is going, so wait for the lock. Any other answer (401, 5xx) means
    // the API is up but unusable and no drain started: go straight to the kill.
    let reply = post_shutdown(&run, &budget);
    let draining = reply.as_ref().is_none_or(|r| r.status == 202);
    if draining && lifecycle::wait_lock_free(state, drain_wait(timeout, reply.as_ref())) {
        println!("stopped");
        return Ok(());
    }
    let Some(pid) = run.pid else {
        if matches!(lifecycle::lock_probe(state), Ok(lifecycle::Liveness::Free)) {
            println!("stopped");
            return Ok(());
        }
        return Err(err(
            "kaanal did not stop and run.json has no pid; not killing anything",
        ));
    };
    match lifecycle::force_stop(
        state,
        &run.token,
        pid,
        std::process::id(),
        lifecycle::verify_pid,
        lifecycle::kill_pid,
    )? {
        lifecycle::Forced::Stopped => println!("stopped"),
        lifecycle::Forced::Killed => println!("stopped (killed pid {pid} after {budget})"),
    }
    Ok(())
}

/// **Private.** `kaanal down --endpoint` (E6): the same `POST /__kaanal/shutdown`, then poll health
/// until nothing answers, for the budget plus 2 s. No run.json, lock or pid, and never a kill: a
/// remote instance that keeps answering is reported, not forced.
fn down_remote(t: &target::Target, timeout: Duration) -> Result<(), Fail> {
    let budget = humantime::format_duration(timeout).to_string();
    let agent = control_agent(Duration::from_secs(5));
    let mut r = agent
        .post(&t.url("/__kaanal/shutdown"))
        .header("Host", &t.host_header())
        .header("Authorization", &t.auth())
        .send_json(json!({ "timeout": budget }))
        .map_err(|e| t.unreachable(&clean(&e.to_string())))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 202 {
        let msg = v["error"].as_str().unwrap_or(&text);
        return Err(t.refused(status, msg).unwrap_or_else(|| {
            err(clean(&format!(
                "POST /__kaanal/shutdown: HTTP {status}: {msg}"
            )))
        }));
    }
    // As local `down`: a drain already running with more time left is waited for.
    let left = v["timeout_ms"]
        .as_u64()
        .map(|ms| lifecycle::server_wait(Some(ms), 0));
    let wait = drain_wait(timeout, Some(&ShutdownReply { status, left }));
    let deadline = Instant::now() + wait;
    loop {
        // Only a refused or dropped connection means gone; a slow or odd answer means still up.
        let left = deadline.saturating_duration_since(Instant::now());
        // At least 2.5 s: with less left, ureq reports the global timeout before the 2 s connect
        // timeout, and a closed port on Windows would look like a live instance.
        let res = control_agent(left.clamp(Duration::from_millis(2500), Duration::from_secs(5)))
            .get(&t.url("/__kaanal/health"))
            .header("Host", &t.host_header())
            .header("Authorization", &t.auth())
            .call();
        // Windows reports a closed port as a connect timeout (its refusal takes longer than ours).
        let gone = match res {
            Err(ureq::Error::Io(e)) => matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ),
            Err(ureq::Error::Timeout(ureq::Timeout::Connect)) => true,
            _ => false,
        };
        if gone {
            println!("stopped");
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(err(format!(
                "{} is still answering after {}; not killing a remote instance",
                t.base(),
                humantime::format_duration(wait)
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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
    let mut body = json!({ "chaos": cfg.chaos, "pins": cfg.pins });
    if let Some(d) = &cfg.description {
        body["description"] = json!(d);
    }
    body
}

/// **Private.** A scenario name checked here, so a reserved one (`active`, `save`, `reset`) is
/// refused with a message instead of hitting another route.
fn named(name: &str) -> Result<&str, Fail> {
    valid_name(name).map_err(Fail::usage)?;
    Ok(name)
}

/// **Private.** `--from` as the API's tagged form: `live`, `effective` or `scenario:NAME`.
fn save_from(from: &str) -> Result<Value, Fail> {
    match from {
        "live" | "effective" => Ok(json!(from)),
        f => match f.strip_prefix("scenario:") {
            Some(n) if !n.is_empty() => Ok(json!({ "scenario": n })),
            _ => Err(Fail::usage(format!(
                "bad --from '{}': use live, effective or scenario:NAME",
                clean(f)
            ))),
        },
    }
}

/// **Private.** The `scenario show` text of one definition.
fn print_scenario(v: &Value) {
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
    for p in v["pins"].as_array().into_iter().flatten() {
        let extra = pin_extra(&p["status"], p["delay"].as_str().map(str::to_string));
        let line = format!(
            "  pin {}: {}{extra}",
            p["endpoint"].as_str().unwrap_or(""),
            p["state"].as_str().unwrap_or("")
        );
        println!("{}", clean(&line));
    }
}

/// **Private.** `prefix` + `active scenarios: a, b` (or the stack answer under `--json`).
fn say_active(v: &Value, prefix: &str) {
    output::emit(v, || {
        println!("{prefix}active scenarios: {}", active_names(v).join(", "));
    });
}

/// **Private.** `kaanal scenario ...`: thin calls to the control API, except `load`, which
/// parses the file here so a mistake shows as `file:line:col`.
fn scenario(cmd: ScenarioCmd) -> Result<(), Fail> {
    match cmd {
        ScenarioCmd::List => {
            let v = call("GET", "/__kaanal/scenarios", None)?;
            output::emit(&v, || {
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
            });
        }
        ScenarioCmd::Show { name } => {
            let v = call("GET", &scenario_path(named(&name)?, ""), None)?;
            output::emit(&v, || print_scenario(&v));
        }
        ScenarioCmd::Activate { name, ttl } => {
            let body = ttl_body("ttl", ttl)?;
            let v = call("POST", &scenario_path(named(&name)?, "/activate"), body)?;
            say_active(&v, "");
        }
        ScenarioCmd::Deactivate { name } => {
            let name = named(&name)?;
            // ponytail: the answer is a bare 204, so "was it active" is read just before; a
            // concurrent change in between can make this line wrong, never the state.
            let was = call("GET", "/__kaanal/scenarios/active", None)?;
            let v = call("POST", &scenario_path(name, "/deactivate"), None)?;
            output::emit(&v, || {
                if active_names(&was).contains(&name) {
                    println!("deactivated {name}");
                } else {
                    println!("ok: {name} is not active");
                }
            });
        }
        ScenarioCmd::Set { names } => {
            names.iter().try_for_each(|n| named(n).map(drop))?;
            let v = call("PUT", "/__kaanal/scenarios/active", Some(json!(names)))?;
            say_active(&v, "");
        }
        ScenarioCmd::Clear => {
            let v = call("DELETE", "/__kaanal/scenarios", None)?;
            output::emit(&v, || println!("cleared"));
        }
        ScenarioCmd::Active => {
            let v = call("GET", "/__kaanal/scenarios/active", None)?;
            output::emit(&v, || {
                for n in active_names(&v) {
                    println!("{}", clean(n));
                }
            });
        }
        ScenarioCmd::Load { file } => {
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Fail::usage(format!("{}: no usable file name", file.display())))?;
            valid_name(name).map_err(|m| Fail::usage(format!("{}: {m}", file.display())))?;
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
            say_active(&v, &format!("loaded {name}; "));
        }
        ScenarioCmd::Save {
            name,
            from,
            force,
            clear_live,
        } => {
            named(&name)?;
            let from = save_from(&from)?;
            let body =
                json!({ "name": name, "from": from, "force": force, "clear_live": clear_live });
            let v = call("POST", "/__kaanal/scenarios/save", Some(body))?;
            output::emit(&v, || {
                println!("saved {}", clean(v["path"].as_str().unwrap_or("")));
            });
            warn_all(&v);
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
    let root = std::env::var("SystemRoot").map_err(|_| "SystemRoot is not set")?;
    // Full paths, so an icacls.exe or whoami.exe planted earlier on PATH is never run.
    let who = lifecycle::os_command(format!(r"{root}\System32\whoami.exe"))
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
    let out = lifecycle::os_command(&icacls)
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
    let out = lifecycle::os_command(&icacls)
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
    lock_at(Path::new(LOCK_FILE), 10, Duration::from_millis(20))
}

/// **Private.** [`lock_instance`] on `path`. A status probe holds a shared lock for an instant, so
/// `WouldBlock` is retried `tries` times, `gap` apart, before it is called "already running"; a live
/// instance holds the lock forever, so a true "already running" costs about `tries * gap`.
fn lock_at(path: &Path, tries: u32, gap: Duration) -> Result<std::fs::File, Fail> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    debug_assert!(tries > 0, "at least one attempt");
    for left in (0..tries.max(1)).rev() {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::WouldBlock) if left > 0 => std::thread::sleep(gap),
            Err(std::fs::TryLockError::WouldBlock) => break,
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(err(format!("{}: {e}", path.display())));
            }
        }
    }
    let port = path
        .parent()
        .and_then(lifecycle::read_run)
        .map_or(String::new(), |r| format!(" on port {}", r.addr.port()));
    Err(err(format!(
        "kaanal is already running{port} (this directory's {LOCK_FILE} is held)"
    )))
}

/// **Private.** Writes `{host, port, token, pid, startup}` via temp file + rename, so a concurrent
/// `status` never reads half a file. `host` is where the CLI connects: the bound address,
/// or loopback when bound to all interfaces.
fn write_run_file(bound: SocketAddr, token: &str, startup: &Value) -> Result<(), Fail> {
    use std::io::Write;
    let host = match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let body = json!({
        "host": host, "port": bound.port(), "token": token, "pid": std::process::id(),
        "startup": startup,
    })
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

/// **Private.** Calls the instance [`target::resolve`] picks: `--endpoint` / `KAANAL_ENDPOINT`, else
/// the one found via `.kaanal/run.json`. Locally exit 2 if it isn't running or its state is stale,
/// exit 1 if it is alive but silent (spec M9, M16); remotely no answer is exit 1 (E5). The client
/// never follows redirects and never uses a proxy — either could forward the token.
fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, Fail> {
    let target = target::resolve()?;
    let auth = target.auth();
    let url = target.url(path);
    // The control API only accepts a loopback Host with the bound port (R5.3), even when
    // we connect to a non-loopback or remote address.
    let host = target.host_header();
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
    let mut resp = res.map_err(|e| target.unreachable(&e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 300 {
        let msg = v["error"].as_str().unwrap_or(&text);
        if let Some(f) = target.refused(status, msg) {
            return Err(f);
        }
        let detail = v["detail"]
            .as_str()
            .map_or(String::new(), |d| format!(" ({d})"));
        return Err(err(clean(&format!(
            "{method} {path}: HTTP {status}: {msg}{detail}"
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
    // Only the text table shows pins; fetched before any output so a failure prints nothing.
    let pins = if output::json_on() {
        Value::Null
    } else {
        call("GET", "/__kaanal/pins", None)?
    };
    let live = call("GET", "/__kaanal/chaos", None)?;
    let rows = output::node_rows(&topo, &active, &live);
    output::emit(&Value::Array(rows.clone()), || {
        output::print_node_table(&rows);
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
        for (key, p) in pins["effective"].as_object().into_iter().flatten() {
            let line = format!(
                "pin {key}: {}{} ({})",
                p["state"].as_str().unwrap_or(""),
                pin_extra_ms(p),
                source_text(&p["source"])
            );
            println!("{}", clean(&line));
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
    });
    Ok(())
}

/// **Private.** `kaanal logs`: checks the flags and turns them into the server's query pairs.
fn logs_cmd(a: &LogsArgs) -> Result<(), Fail> {
    if !(1..=1000).contains(&a.n) {
        return Err(Fail::usage("-n must be from 1 to 1000"));
    }
    if a.scope.as_deref() == Some("") {
        // ponytail: the server can filter the empty-header scope, the CLI cannot ask for it yet
        return Err(Fail::usage("--scope needs a scope name"));
    }
    let params: Vec<(String, String)> = [
        Some(("limit", a.n.to_string())),
        a.since.clone().map(|v| ("since", v)),
        a.endpoint.clone().map(|v| ("endpoint", v)),
        a.status.clone().map(|v| ("status", v)),
        a.failed.then(|| ("failed", "true".to_string())),
        a.scope.clone().map(|v| ("scope", v)),
    ]
    .into_iter()
    .flatten()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    // A bad filter is a usage error in both modes, before any request or run-file read.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let filter = logview::filter_of(&params, now_ms).map_err(Fail::usage)?;
    if a.follow {
        return follow_logs(&params, &filter, output::json_on());
    }
    logs(&params, output::json_on())
}

/// **Private.** Prints one line to stdout. A closed pipe (`kaanal logs | head`) ends the process
/// quietly with exit 0, which is how every well-behaved filter tool treats it.
fn out_line(text: &str) {
    out_bytes(
        format!(
            "{text}
"
        )
        .as_bytes(),
    );
}

/// **Private.** Writes raw bytes to stdout with the same closed-pipe rule as [`out_line`].
fn out_bytes(bytes: &[u8]) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(bytes).and_then(|()| out.flush()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("kaanal: cannot write to stdout: {e}");
        std::process::exit(1);
    }
}

/// **Private.** `kaanal trace`: the waterfall of one request, or its entry as JSON.
fn trace(key: &str) -> Result<(), Fail> {
    let e = call(
        "GET",
        &format!("/__kaanal/trace/{}", encode_segment(key)),
        None,
    )
    // `last` is unknown only while nothing has finished yet
    .map_err(|f| match f {
        Fail(1, m) if key == "last" && m.contains("HTTP 404") => err("no requests recorded yet"),
        f => f,
    })?;
    out_line(&if output::json_on() {
        e.to_string()
    } else {
        logview::waterfall(&e)
    });
    Ok(())
}

/// **Private.** A status notice of `logs -f`. With `--json` stdout carries JSON Lines only, so
/// notices go to stderr then.
fn notice(json: bool, text: &str) {
    if json {
        eprintln!("{text}");
    } else {
        out_line(text);
    }
}

/// **Private.** One log line, or the JSON object with `--json`.
fn show(v: &Value, json: bool) {
    out_line(&if json {
        v.to_string()
    } else {
        logview::line(v)
    });
}

/// **Private.** `/__kaanal/logs?k=v&...` for the filter pairs.
fn logs_path(params: &[(String, String)]) -> String {
    let q: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode_segment(v)))
        .collect();
    format!("/__kaanal/logs?{}", q.join("&"))
}

/// **Private.** `kaanal logs`: the tail, then (`-f`) every request as it finishes.
fn logs(params: &[(String, String)], json: bool) -> Result<(), Fail> {
    let page = call("GET", &logs_path(params), None)?;
    for e in page["entries"].as_array().into_iter().flatten() {
        show(e, json);
    }
    Ok(())
}

/// **Private.** Ctrl+C is a normal way to stop following: exit 0.
fn exit_on_ctrl_c() {
    std::thread::spawn(|| {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(tokio::signal::ctrl_c());
            std::process::exit(0);
        }
    });
}

/// **Private.** `logs -f`. Subscribe first and let the reader thread buffer events, then fetch the
/// tail, then print the tail and everything buffered or live that the tail did not already
/// contain: a request that finishes between the two is in both and printed once, one that
/// finishes late (slow) is printed when it finishes. Live events are filtered here, with the
/// server's own filter grammar.
fn follow_logs(
    params: &[(String, String)],
    filter: &kaanal_server::Filter,
    json: bool,
) -> Result<(), Fail> {
    let target = target::resolve()?;
    let gone = |why: String| target.unreachable(&why);
    exit_on_ctrl_c();
    // Its own agent: no overall timeout, the stream is meant to stay open.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .build()
        .into();
    let mut resp = agent
        .get(&target.url("/__kaanal/events"))
        .header("Host", &target.host_header())
        .header("Authorization", &target.auth())
        .call()
        .map_err(|e| gone(e.to_string()))?;
    let status = resp.status().as_u16();
    if status != 200 {
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Err(target
            .refused(status, v["error"].as_str().unwrap_or(""))
            .unwrap_or_else(|| err(format!("GET /__kaanal/events: HTTP {status}"))));
    }
    // Bounded: a follower that cannot keep up loses events visibly (a `lagged` notice), not memory.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(String, String)>>(logview::BACKLOG);
    let dropped_in = Arc::new(AtomicU64::new(0));
    let dropped = Arc::clone(&dropped_in);
    std::thread::spawn(move || {
        let mut r = std::io::BufReader::new(resp.into_body().into_reader());
        let (mut sse, mut buf) = (logview::Sse::default(), Vec::new());
        let dropped = Arc::clone(&dropped_in);
        loop {
            buf.clear();
            match logview::read_capped(&mut r, &mut buf, logview::MAX_MSG) {
                Ok(logview::Line::Eof) | Err(_) => break,
                Ok(logview::Line::TooLong) => sse.overflow(),
                Ok(logview::Line::Text) => {
                    if let Some(m) = sse.push(&logview::lossy_line(&buf))
                        && !logview::forward(&tx, &dropped, m)
                    {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(None);
    });
    let mut seen = logview::Seen::new();
    let page = call("GET", &logs_path(params), None)?;
    for e in page["entries"].as_array().into_iter().flatten() {
        seen.insert(e["id"].as_str().unwrap_or(""));
        show(e, json);
    }
    // Ends only when the stream does (or the printer's queue is cut): the server is gone.
    let mut shutdown_ms: Option<Option<u64>> = None;
    logview::drain(
        &rx,
        &dropped,
        Duration::from_millis(500),
        &mut |s| notice(json, s),
        &mut |event, data| {
            if event == "shutdown" {
                // The server announces its drain; the time left says how long to wait for the exit.
                shutdown_ms = Some(
                    serde_json::from_str::<Value>(data)
                        .ok()
                        .and_then(|v| v["timeout_ms"].as_u64()),
                );
                return;
            }
            if event != "request" {
                return;
            }
            let Ok(mut v) = serde_json::from_str::<Value>(data) else {
                return;
            };
            if !seen.insert(v["id"].as_str().unwrap_or("")) || !logview::passes(filter, &v) {
                return;
            }
            // --json wants the whole entry; the event is only its summary.
            if json {
                let id = v["id"].as_str().unwrap_or("");
                match call(
                    "GET",
                    &format!("/__kaanal/trace/{}", encode_segment(id)),
                    None,
                ) {
                    Ok(full) => v = full,
                    Err(_) => notice(json, &format!("-- could not fetch {} --", clean(id))),
                }
            }
            show(&v, json);
        },
    );
    let drain = shutdown_ms.map(|ms| lifecycle::server_wait(ms, 5000));
    Err(stream_ended(&target, drain))
}

/// **Private.** Why `logs -f` stopped, once the event stream ended; `drain` is the shutdown the
/// server announced, if it did.
fn stream_ended(target: &target::Target, drain: Option<Duration>) -> Fail {
    // Remote: no lock to wait on, and exit 2 ("no local instance") is local-only (E5).
    if target.remote {
        return match drain {
            Some(_) => err(format!("{}: stopped (the server shut down)", target.base())),
            None => target.unreachable("the event stream closed"),
        };
    }
    let state = Path::new(STATE_DIR);
    if let Some(ms) = drain {
        // A `down` (or signal) drain: the process is still finishing its exit. That is a stop.
        lifecycle::wait_lock_free(state, ms + Duration::from_secs(2));
        return Fail(2, "kaanal stopped (the server shut down)".into());
    }
    // No shutdown message: killed, or cut by a tiny drain budget before it was written. Give the
    // exit a moment; a lock that is still held is then honestly "running but not answering".
    lifecycle::wait_lock_free(state, Duration::from_secs(2));
    target.unreachable("the event stream closed")
}

/// **Private.** The pin states `cycle --state` accepts, in cycle order.
const PIN_STATES: [&str; 5] = ["loading", "empty", "error", "slow", "success"];

/// **Private.** ` (status 404, delay 2s)` for the parts that are set, else nothing.
fn pin_extra(status: &Value, delay: Option<String>) -> String {
    let parts: Vec<String> = status
        .as_u64()
        .map(|s| format!("status {s}"))
        .into_iter()
        .chain(delay.map(|d| format!("delay {d}")))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// **Private.** [`pin_extra`] for a server pin object, whose delay is `delay_ms`.
fn pin_extra_ms(p: &Value) -> String {
    let delay = p["delay_ms"]
        .as_u64()
        .map(|ms| humantime::format_duration(Duration::from_millis(ms)).to_string());
    pin_extra(&p["status"], delay)
}

/// **Private.** `live` or `scenario:NAME` for a pin's `source`.
fn source_text(s: &Value) -> String {
    s["scenario"]
        .as_str()
        .map_or_else(|| "live".to_string(), |n| format!("scenario:{n}"))
}

/// **Private.** `kaanal cycle`: one `POST /cycle`, or `--off` as a `DELETE`. State names and the
/// delay are checked here so a typo sends nothing; ranges are the server's.
fn cycle(
    endpoint: &str,
    state: Option<String>,
    status: Option<u16>,
    delay: Option<String>,
    off: bool,
) -> Result<(), Fail> {
    if off {
        let path = format!("/__kaanal/pins?endpoint={}", encode_segment(endpoint));
        // The server answers 204 whether or not a pin existed, so this prints either way.
        let v = call("DELETE", &path, None)?;
        output::emit(&v, || println!("{} -> unpinned", clean(endpoint)));
        return Ok(());
    }
    let mut body = json!({ "endpoint": endpoint });
    if let Some(s) = state {
        if !PIN_STATES.contains(&s.as_str()) {
            return Err(Fail::usage(format!(
                "bad --state '{}': use {}",
                clean(&s),
                PIN_STATES.join(", ")
            )));
        }
        body["state"] = json!(s);
    }
    if let Some(d) = delay {
        let d = d.trim();
        humantime::parse_duration(d)
            .map_err(|_| Fail::usage(format!("bad --delay '{}' (use e.g. 2s)", clean(d))))?;
        body["delay"] = json!(d);
    }
    if let Some(s) = status {
        body["status"] = json!(s);
    }
    let v = call("POST", "/__kaanal/cycle", Some(body))?;
    output::emit(&v, || {
        let line = format!(
            "{} -> {}{}",
            v["endpoint"].as_str().unwrap_or(""),
            v["state"].as_str().unwrap_or(""),
            pin_extra_ms(&v)
        );
        println!("{}", clean(&line));
    });
    Ok(())
}

/// **Private.** `kaanal pins`: the live pins, then the effective pin of each endpoint with
/// its source.
fn pins() -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/pins", None)?;
    output::emit(&v, || pins_text(&v));
    Ok(())
}

/// **Private.** The text of `kaanal pins`.
fn pins_text(v: &Value) {
    let live = v["live"].as_array().map_or(&[][..], Vec::as_slice);
    let eff = v["effective"].as_object();
    if live.is_empty() && eff.is_none_or(serde_json::Map::is_empty) {
        println!("no pins");
        return;
    }
    if live.is_empty() {
        println!("no live pins");
    } else {
        println!("live pins:");
    }
    for p in live {
        let line = format!(
            "  {:<24} {}{}",
            p["endpoint"].as_str().unwrap_or(""),
            p["state"].as_str().unwrap_or(""),
            pin_extra_ms(p)
        );
        println!("{}", clean(&line));
    }
    println!("effective pins:");
    for (key, p) in eff.into_iter().flatten() {
        let line = format!(
            "  {key:<24} {:<8} {}{}",
            p["state"].as_str().unwrap_or(""),
            source_text(&p["source"]),
            pin_extra_ms(p)
        );
        println!("{}", clean(&line));
    }
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
        .map(|c| if c.is_control() || spoofs(c) { '?' } else { c })
        .collect()
}

/// **Private.** Invisible characters that reorder or hide text without being control characters:
/// bidi marks and overrides, line/paragraph separators, word joiner and invisible operators,
/// and the byte order mark. (Zero-width joiners stay: emoji and some scripts need them.)
fn spoofs(c: char) -> bool {
    matches!(
        c,
        '\u{61c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
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

/// **Private.** The activate body for `--ttl` or `--for`, validated here so a typo sends nothing;
/// `what` names the flag in the error.
fn ttl_body(what: &str, ttl: Option<String>) -> Result<Option<Value>, Fail> {
    ttl.map(|t| {
        let t = t.trim();
        humantime::parse_duration(t)
            .ok()
            .filter(|d| (Duration::from_millis(100)..=Duration::from_hours(24)).contains(d))
            .ok_or_else(|| Fail::usage(format!("bad {what} '{t}' (100ms to 24h, e.g. 30s)")))?;
        Ok(json!({ "ttl": t }))
    })
    .transpose()
}

/// **Private.** ` (active)` or ` (active, expires in X)` for one list row, else nothing.
fn active_suffix(s: &Value) -> String {
    match (s["active"] == true, s["expires_in_ms"].as_u64()) {
        (true, Some(ms)) => format!(" (active, expires in {})", output::remaining(ms)),
        (true, None) => " (active)".to_string(),
        _ => String::new(),
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
    #[test]
    fn down_waits_for_the_longer_of_its_budget_and_an_earlier_drains_time_left() {
        use super::{ShutdownReply, drain_wait};
        use std::time::Duration;
        let s = Duration::from_secs;
        let reply = |left| ShutdownReply { status: 202, left };
        assert_eq!(drain_wait(s(5), None), s(7));
        assert_eq!(drain_wait(s(5), Some(&reply(None))), s(7));
        assert_eq!(drain_wait(s(5), Some(&reply(Some(s(60))))), s(62));
        assert_eq!(drain_wait(s(30), Some(&reply(Some(s(2))))), s(32));
    }

    #[test]
    fn exposure_text_is_the_exact_single_spaced_message() {
        assert_eq!(
            super::exposure_text("0.0.0.0".parse().unwrap()),
            "warning: binding 0.0.0.0 makes the mock and control ports reachable from your network; control calls still need the token and Host localhost/127.0.0.1/[::1]"
        );
    }

    use super::{clean, encode_segment, lock_at, parse_ms, parse_rate, ttl_body};
    use std::time::Duration;

    /// A scratch dir removed on drop, so a failing assert leaves nothing behind.
    struct Tmp(std::path::PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn lock_dir(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("kaanal-lockat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    #[test]
    fn lock_at_waits_out_a_brief_probe() {
        let d = lock_dir("probe");
        let path = d.0.join("lock");
        let probe = std::fs::File::create(&path).unwrap();
        probe.try_lock_shared().unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            probe.unlock().unwrap();
        });
        assert!(lock_at(&path, 50, Duration::from_millis(20)).is_ok());
        t.join().unwrap();
    }

    #[test]
    fn lock_at_reports_a_held_lock_as_already_running() {
        let d = lock_dir("held");
        let path = d.0.join("lock");
        let holder = std::fs::File::create(&path).unwrap();
        holder.try_lock().unwrap();
        let start = std::time::Instant::now();
        let e = lock_at(&path, 5, Duration::from_millis(20)).unwrap_err();
        assert!(e.1.contains("already running"), "{}", e.1);
        assert!(
            start.elapsed() >= Duration::from_millis(80),
            "it must retry first"
        );
        drop(holder);
    }

    #[test]
    fn ttl_body_sends_the_trimmed_value() {
        let b = ttl_body("ttl", Some(" 30s ".into()))
            .ok()
            .flatten()
            .unwrap();
        assert_eq!(b["ttl"], "30s");
        assert!(ttl_body("ttl", Some("soon".into())).is_err());
        assert!(matches!(ttl_body("ttl", None), Ok(None)));
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
        // text reordering and invisible characters are neutralised too
        assert_eq!(
            clean("a\u{202e}b\u{2066}c\u{200f}d\u{61c}e\u{2028}f\u{feff}"),
            "a?b?c?d?e?f?"
        );
        // joiners (emoji, scripts) and ordinary non-ASCII text stay
        assert_eq!(
            clean("\u{1f468}\u{200d}\u{1f469} \u{65e5}\u{672c}"),
            "\u{1f468}\u{200d}\u{1f469} \u{65e5}\u{672c}"
        );
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_segment("db-1_a.b~"), "db-1_a.b~");
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_segment("\u{1b}"), "%1B");
        assert_eq!(encode_segment("\u{e9}"), "%C3%A9");
    }
}
