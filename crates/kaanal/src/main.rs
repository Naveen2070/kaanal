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
use serde_json::{Value, json};
mod cli;
mod commands;
mod instance;
#[cfg(test)]
mod tests;
mod util;

// The modules are folders by role; their names are re-exported here so a command reaches
// `lifecycle::...` or `output::...` by its short path.
use self::{
    cli::{Cli, Cmd, LogsArgs, ScenarioCmd, completions, schema},
    commands::{
        chaos::{chaos, chaos_text, cycle, node, pin_extra, pin_extra_ms, pins, source_text},
        demo,
        down::{down, down_remote, drain_budget, warn_all},
        init,
        logs::{logs_cmd, out_bytes, out_line, trace},
        scenario::{active_names, endpoints, scenario},
        serve::{Started, drain_wait, serve_locked, start_budget, up, up_detached},
    },
    instance::{
        keys, lifecycle,
        remote::{ShutdownReply, call, control_agent, post_shutdown, status},
        state_dir::{lock_instance, private_dir, write_run_file},
        target,
    },
    util::{
        load::{auth_users, config_err, load, located_err},
        logview, output,
        parse::{active_suffix, clean, clean_lines, encode_segment, ttl_body},
    },
};

/// **Private.** Per-project state directory holding `run.json` and `lock`.
const STATE_DIR: &str = ".kaanal";

/// **Private.** Port + token + pid of the running instance.
const RUN_FILE: &str = ".kaanal/run.json";

/// **Private.** OS-locked file enforcing one instance per directory (rule R6.7).
const LOCK_FILE: &str = ".kaanal/lock";

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
    let cli = match Cli::try_parse().and_then(Cli::fix_route) {
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

/// **Private.** Subcommand dispatch.
fn run(cli: Cli) -> Result<(), Fail> {
    match cli.cmd {
        Cmd::Validate { config } => {
            let (cfg, dir, src) = load(&config)?;
            let warnings =
                kaanal_server::check(&cfg, &dir).map_err(|e| located_err(&config, &src, &e))?;
            let sampled =
                kaanal_server::selfcheck(&cfg, &dir).map_err(|e| located_err(&config, &src, &e))?;
            for w in warnings.into_iter().chain(sampled) {
                eprintln!("warning: {}", clean(&w));
            }
            println!("ok: {}", config.display());
            Ok(())
        }
        Cmd::Init(args) => {
            no_endpoint("init writes local files and takes no endpoint")?;
            let stdin = std::io::stdin();
            let interactive = std::io::IsTerminal::is_terminal(&stdin);
            init::init(
                &args,
                &mut stdin.lock(),
                &mut std::io::stderr(),
                &mut std::io::stdout(),
                interactive,
            )
        }
        Cmd::Completions { shell } => {
            completions(shell);
            Ok(())
        }
        Cmd::Schema { kind } => schema(kind),
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
        Cmd::Demo(args) => {
            no_endpoint("demo starts a local instance and takes no endpoint")?;
            demo::demo(&args)
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
            route,
            state,
            status,
            delay,
            off,
        } => cycle(&route, state, status, delay, off),
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

/// **Private.** First line of every block `kaanal endpoints` adds (D22).
const SKELETON_HEADER: &str = "# Added by kaanal endpoints";

/// **Private.** The file `kaanal endpoints --write` creates when `endpoints:` is absent.
const DEFAULT_ENDPOINTS_FILE: &str = "kaanal.endpoints.yaml";

/// **Private.** The pin states `cycle --state` accepts, in cycle order.
const PIN_STATES: [&str; 5] = ["loading", "empty", "error", "slow", "success"];

/// **Private.** RFC 3986 unreserved characters; everything else in a path segment is escaped.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
