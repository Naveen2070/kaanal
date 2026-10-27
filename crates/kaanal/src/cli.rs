//! The command-line shape: clap types for every subcommand, the `Cli` parse fix-ups, `schema` and `completions`.

use super::{
    Args, CommandFactory, Duration, Fail, IpAddr, Parser, PathBuf, Subcommand, demo, drain_budget,
    err, init, out_bytes, start_budget,
};

/// **Private.** Top-level CLI shape for clap.
#[derive(Parser)]
#[command(
    name = "kaanal",
    version,
    about = "A mirage of your backend: mock APIs, chaos and simulation"
)]
pub(crate) struct Cli {
    /// Print read commands' results as JSON (also `KAANAL_JSON=1`); errors stay text on stderr
    #[arg(long, global = true)]
    pub(crate) json: bool,
    /// Talk to the instance at http://HOST:PORT instead of the one in .kaanal/ (also
    /// `KAANAL_ENDPOINT`); may go before or after the subcommand
    #[arg(long, global = true, value_name = "URL")]
    pub(crate) endpoint: Option<String>,
    /// Token for --endpoint. Prefer `KAANAL_TOKEN`: a value given here is visible in process
    /// lists and shell history
    #[arg(long, global = true, value_name = "TOKEN")]
    pub(crate) token: Option<String>,
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

/// **Private.** Subcommands.
#[derive(Subcommand)]
pub(crate) enum Cmd {
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
    /// Print a JSON Schema (point your editor's YAML plugin at it): kaanal.yaml by default,
    /// or a data.yaml dictionary or a locales/<tag>.yaml pack
    Schema {
        #[arg(value_enum, default_value_t = SchemaKind::Config)]
        kind: SchemaKind,
    },
    /// Create a starter kaanal.yaml from an `OpenAPI` 3.0 file
    Init(init::InitArgs),
    /// Run a demo shop with a chaos bar in your browser (writes only .kaanal/; Ctrl-C to stop)
    Demo(demo::DemoArgs),
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
        route: String,
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
pub(crate) enum NodeCmd {
    /// Take a node down (503 for every request), e.g. `kaanal node down db --for 30s`
    Down {
        node: String,
        /// Only this replica (0-based) of the node
        #[arg(long)]
        replica: Option<u16>,
        /// Bring it back after this long (100ms to 24h, e.g. 30s)
        #[arg(long = "for", value_name = "DURATION")]
        for_: Option<String>,
    },
    /// List nodes with their up/down state and chaos
    List,
    /// Clear a node's down flag (other chaos stays); fails if a scenario or override still holds it down
    Up {
        node: String,
        /// Only this replica (0-based) of the node
        #[arg(long)]
        replica: Option<u16>,
    },
}

/// **Private.** Flags of `kaanal logs`.
#[derive(Args)]
pub(crate) struct LogsArgs {
    /// How many of the latest requests to show (1-1000)
    #[arg(short = 'n', default_value_t = 50)]
    pub(crate) n: u32,
    /// Keep printing new requests as they finish (Ctrl+C to stop)
    #[arg(short, long)]
    pub(crate) follow: bool,
    /// Only requests newer than this, e.g. 5m or an RFC 3339 time
    #[arg(long)]
    pub(crate) since: Option<String>,
    /// Route template as logged, e.g. "GET /users/{id}"
    #[arg(long)]
    pub(crate) route: Option<String>,
    /// Statuses, e.g. 404 or 404,5xx
    #[arg(long)]
    pub(crate) status: Option<String>,
    /// Only requests a node failed
    #[arg(long)]
    pub(crate) failed: bool,
    /// Only requests that ran under this scenario scope
    #[arg(long)]
    pub(crate) scope: Option<String>,
}

/// **Private.** `chaos` subcommands.
#[derive(Subcommand)]
pub(crate) enum ChaosCmd {
    /// Set chaos on a node, an endpoint, or a node only when reached through an endpoint,
    /// e.g. `kaanal chaos set db --error-rate 100% --error 503` or `--route "GET /orders" --down`
    Set {
        /// The node (omit it to set chaos on the whole `--route`)
        node: Option<String>,
        /// Endpoint key, e.g. "GET /orders"; with a node: that node, only through this endpoint
        #[arg(long)]
        route: Option<String>,
        /// One replica (0-based) of the node, whatever the endpoint
        #[arg(long, requires = "node", conflicts_with = "route")]
        replica: Option<u16>,
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
    /// Clear chaos on one node, one endpoint (`--route`), or both together; none means everything
    Clear {
        node: Option<String>,
        /// Endpoint key, e.g. "GET /orders"
        #[arg(long)]
        route: Option<String>,
        /// One replica (0-based) of the node
        #[arg(long, requires = "node", conflicts_with = "route")]
        replica: Option<u16>,
    },
}

/// **Private.** `scenario` subcommands.
#[derive(Subcommand)]
pub(crate) enum ScenarioCmd {
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

impl Cli {
    /// **Private.** clap cannot hold a hidden `--endpoint` alias of `--route` next to the global
    /// `--endpoint URL`, so the old route-key spelling (`logs|chaos set|chaos clear --endpoint
    /// "GET /x"`) arrives as the global flag. A route key is `METHOD /path` and has a space; an
    /// instance address never does. Such a value moves to the route slot of those subcommands
    /// (R1.48). Also enforces that `chaos set` has a node or a route.
    pub(crate) fn fix_route(mut self) -> Result<Self, clap::Error> {
        let slot = match &mut self.cmd {
            Cmd::Logs(a) => Some(&mut a.route),
            Cmd::Chaos {
                cmd: ChaosCmd::Set { route, .. } | ChaosCmd::Clear { route, .. },
            } => Some(route),
            _ => None,
        };
        if let Some(slot) = slot
            && slot.is_none()
            && self.endpoint.as_deref().is_some_and(|v| v.contains(' '))
        {
            *slot = self.endpoint.take();
        }
        if let Cmd::Chaos {
            cmd:
                ChaosCmd::Set {
                    node: None,
                    route: None,
                    ..
                },
        } = &self.cmd
        {
            return Err(Cli::command().error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "chaos set needs a node or --route",
            ));
        }
        Ok(self)
    }
}

/// **Private.** Which file `kaanal schema` describes.
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum SchemaKind {
    /// kaanal.yaml
    Config,
    /// data.yaml, the field dictionary
    Dictionary,
    /// locales/<tag>.yaml, a locale pack
    Pack,
}

/// **Private.** `kaanal schema [config|dictionary|pack]`: that file's JSON Schema.
pub(crate) fn schema(kind: SchemaKind) -> Result<(), Fail> {
    let value = match kind {
        SchemaKind::Config => kaanal_core::config_schema(),
        SchemaKind::Dictionary => kaanal_server::dictionary_schema(),
        SchemaKind::Pack => kaanal_server::pack_schema(),
    };
    let text = serde_json::to_string_pretty(&value).map_err(|e| err(e.to_string()))?;
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
pub(crate) fn completions(shell: clap_complete::Shell) {
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "kaanal", &mut buf);
    out_bytes(&buf);
}
