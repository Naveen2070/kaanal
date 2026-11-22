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
        /// Start with no system and an empty draft: build one with `kaanal draft` or in the UI,
        /// then `draft apply`; `-c` is where `draft save` writes it (the file is not read)
        #[arg(long, conflicts_with_all = ["detach", "detached"])]
        blank: bool,
    },
    /// Start blank and open the browser: build a system in the UI (same as `up --blank`)
    Studio {
        /// Where `draft save` writes the system (the file is not read)
        #[arg(short, long, default_value = "kaanal.yaml")]
        config: PathBuf,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        /// Only print the URL; do not open the browser
        #[arg(long)]
        no_open: bool,
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
    /// Create a starter kaanal.yaml (estimated timings, queues, endpoints) from an `OpenAPI` 3.0 file
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
    /// Show, replace, check and apply the running instance's draft (its `kaanal.yaml` text)
    Draft {
        #[command(subcommand)]
        cmd: DraftCmd,
    },
    /// Run the simulator on the running system and read the report (a relative what-if, not a prediction)
    Sim {
        #[command(subcommand)]
        cmd: SimCmd,
    },
    /// Print one generated response body of an endpoint, e.g. `kaanal sample "GET /users/{id}"`
    Sample {
        /// Endpoint key, "METHOD /path"
        route: String,
        /// Seed of the data (default the config's `seed:`)
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Print the generated skeleton `OpenAPI` file for the draft's `endpoints:`, or write it with -o
    Skeleton {
        /// Write the file here instead of printing it
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite an existing output file
        #[arg(long, requires = "output")]
        force: bool,
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
        /// A link instead of a node: the edge FROM:TO, e.g. `--edge app:db`
        #[arg(long, value_name = "FROM:TO", conflicts_with_all = ["node", "route", "replica"])]
        edge: Option<String>,
        /// Link only: random extra latency of up to this much either way, e.g. 5ms
        #[arg(long, requires = "edge")]
        jitter: Option<String>,
        /// Link only: cap the edge's bandwidth, bytes per second or with a unit like 500KB/s (at most its own)
        #[arg(long = "bandwidth-cap", requires = "edge")]
        bandwidth_cap: Option<String>,
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
        /// A link instead of a node: the edge FROM:TO, e.g. `--edge app:db`
        #[arg(long, value_name = "FROM:TO", conflicts_with_all = ["node", "route", "replica"])]
        edge: Option<String>,
        /// One replica (0-based) of the node
        #[arg(long, requires = "node", conflicts_with = "route")]
        replica: Option<u16>,
    },
}

/// **Private.** `draft` subcommands.
#[derive(Subcommand)]
pub(crate) enum DraftCmd {
    /// Print the draft text
    Show,
    /// Replace the draft with a file's text (nothing is applied)
    Set {
        /// The file whose text becomes the draft
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Check the draft, or a file without storing it; a problem prints with its line and column and exits 1
    Validate {
        /// Check this file's text instead of the stored draft
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    /// Make the draft the running system without a restart
    Apply {
        /// How long to wait for requests in flight to finish (at most 60s)
        #[arg(long, default_value = "10s")]
        timeout: String,
    },
    /// Write the draft to the config file (atomically); the running system is not touched
    Save {
        /// Save even though edits dropped comments or anchors
        #[arg(long)]
        confirm_loss: bool,
        /// What to do with scenario files the draft no longer resolves against (default: leave them)
        #[arg(long, value_enum)]
        fix_scenarios: Option<FixScenarios>,
    },
}

/// **Private.** `sim` subcommands.
#[derive(Subcommand)]
pub(crate) enum SimCmd {
    /// Start a run and wait for its report (one run at a time)
    Run {
        /// A profile of the config's `traffic:` section
        #[arg(short, long, conflicts_with = "file")]
        profile: Option<String>,
        /// A file holding one profile (`rps`, `journeys`, ...) as YAML or JSON
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Simulated time, e.g. 60s (at most 1h; default the profile's, else 60s)
        #[arg(long)]
        duration: Option<String>,
        /// Simulated time before results count (default a tenth of the duration)
        #[arg(long)]
        warmup: Option<String>,
        /// Seed of the draws (default the config's `seed:`)
        #[arg(long)]
        seed: Option<u64>,
        /// Run under this scenario's chaos alone (default: the live chaos)
        #[arg(long)]
        scenario: Option<String>,
        /// Print the run id and return without waiting
        #[arg(long)]
        no_wait: bool,
    },
    /// Raise the load until p99 or errors cross a limit, and print the knee
    Sweep {
        /// A profile of the config's `traffic:` section
        #[arg(short, long, conflicts_with = "file")]
        profile: Option<String>,
        /// A file holding one profile (`rps`, `journeys`, ...) as YAML or JSON
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Highest p99 that passes, e.g. 500ms (default 1s)
        #[arg(long)]
        p99: Option<String>,
        /// Highest share of failed requests that passes, e.g. 1% or 1, 0 to 100 (default 1%)
        #[arg(long)]
        errors: Option<String>,
        /// Top of the ladder of load factors, 1 to 64 (default 8)
        #[arg(long)]
        max_factor: Option<f64>,
        /// Seed of the draws, the same at every point (default the config's `seed:`)
        #[arg(long)]
        seed: Option<u64>,
        /// Sweep under this scenario's chaos alone (default: the live chaos)
        #[arg(long)]
        scenario: Option<String>,
        /// Print the sweep id and return without waiting
        #[arg(long)]
        no_wait: bool,
        /// When it finishes, write its report (Markdown) to this file; not with --no-wait
        #[arg(long, value_name = "FILE", conflicts_with = "no_wait")]
        report: Option<PathBuf>,
        /// Overwrite the --report file if it exists
        #[arg(long, requires = "report")]
        force: bool,
    },
    /// Plan the sizing that holds a target load (a relative what-if); never applied
    Plan(PlanArgs),
    /// List the last eight runs
    List,
    /// Show one run with its report, or a sweep with its finding
    Show {
        /// A run id, e.g. s1
        id: String,
    },
    /// Print the report of a finished sweep: Markdown, or with --json the data alone
    Report {
        /// A sweep id, e.g. s1
        id: String,
        /// Write the report to this file instead of printing it
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite the file if it exists
        #[arg(long, requires = "output")]
        force: bool,
    },
}

/// **Private.** `sim plan` flags.
#[derive(clap::Args)]
pub(crate) struct PlanArgs {
    /// A profile of the config's `traffic:` section (default: the first)
    #[arg(short, long, conflicts_with = "file")]
    pub(crate) profile: Option<String>,
    /// A file holding one profile (`rps`, `journeys`, ...) as YAML or JSON
    #[arg(short, long)]
    pub(crate) file: Option<PathBuf>,
    /// The load to hold: a multiple of the profile's rates (`3x`, 0.1 to 64) or a total req/s (`500`)
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) target: String,
    /// Highest p99 that passes, e.g. 500ms (default 1s)
    #[arg(long)]
    pub(crate) p99: Option<String>,
    /// Highest share of failed requests that passes, e.g. 1% or 1 (default 1%)
    #[arg(long)]
    pub(crate) errors: Option<String>,
    /// The most the plan may cost, dollars an hour of the priced nodes, e.g. 2.00
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) budget: Option<String>,
    /// A node the plan must not move (repeatable)
    #[arg(long, value_name = "NODE")]
    pub(crate) keep: Vec<String>,
    /// The most steps, up to 100 (default 30)
    #[arg(long)]
    pub(crate) max_steps: Option<u32>,
    /// Seed of the draws (default the config's `seed:`)
    #[arg(long)]
    pub(crate) seed: Option<u64>,
    /// Plan under this scenario's chaos alone (default: the live chaos)
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    /// Print the plan id and return without waiting
    #[arg(long)]
    pub(crate) no_wait: bool,
    /// When it finishes, write its report (Markdown) to this file; not with --no-wait
    #[arg(long, value_name = "FILE", conflicts_with = "no_wait")]
    pub(crate) report: Option<PathBuf>,
    /// When it finishes, write the proposed draft (the applied text with the plan's steps) to this file; not with --no-wait
    #[arg(short, long, value_name = "FILE", conflicts_with = "no_wait")]
    pub(crate) output: Option<PathBuf>,
    /// Overwrite the --report and -o files if they exist
    #[arg(long)]
    pub(crate) force: bool,
}

/// **Private.** `draft save --fix-scenarios` values.
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum FixScenarios {
    /// Point them at the renamed node (for a rename made since the draft was set)
    Rewrite,
    /// Delete the files
    Remove,
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
                    edge: None,
                    ..
                },
        } = &self.cmd
        {
            return Err(Cli::command().error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "chaos set needs a node, --route or --edge",
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
