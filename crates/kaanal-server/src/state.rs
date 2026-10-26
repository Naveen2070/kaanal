//! [`AppState`]: everything the running server knows, built once at startup.
//!
//! [`AppState::build`] is the full load pipeline shared by `serve` and `check`
//! (`kaanal validate`): topology → `OpenAPI` specs → endpoints (inline or the endpoints
//! file) compiled by `kaanal_core::chain::compile` → route table. Every problem is a
//! fatal error with a precise message, or — for ignorable schema keywords — a collected
//! warning.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::config::{EndpointsConfig, NodeConfig, SeedSource, parse_endpoints};
use kaanal_core::diagnostics::{Located, PathSeg, keys};
use kaanal_core::{
    ChaosStack, Config, EffectiveChaos, EffectivePins, Endpoint, Endpoints, NodeId, ScenarioSet,
    Source, Topology, resolve_scenario, valid_name,
};

use crate::scenarios;
use kaanal_data::{ApiModel, DEFAULT_LOCALE, DataError, DataSet, Options, PageSpec};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

/// One route: a `method + path` pair and the compiled endpoint that answers it.
pub struct Route {
    /// Uppercase HTTP method.
    pub method: String,
    /// `"GET /users/{id}"`: chain seed material and display name.
    pub key: String,
    /// The compiled chain, op, `override` and `error_map` (HEAD copies included, R6.8).
    pub endpoint: Arc<Endpoint>,
    /// Success status from the spec (200 for a static route); an `override` status wins.
    pub status: u16,
    /// Response schema for generated bodies (`None`: empty body, or a static route).
    pub schema: Option<Value>,
    /// Requests seen so far; becomes `ChainRequest.n`.
    pub counter: Arc<AtomicU64>,
    /// D3: the paging contract of a generated list route; `None` for every other route
    /// (unpaged, `override`, `collection:` or a built-in responder), which answers as before.
    pub paging: Option<Arc<PageSpec>>,
    /// D3: the `total:` of the route's entry; a `HEAD` that follows its `GET` and sets none takes
    /// the `GET`'s. `None` = seeded by the route.
    pub total: Option<u32>,
}

/// One finished request, broadcast to `/__kaanal/events` subscribers.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Our request id (`r04f2-0042`).
    pub id: String,
    /// The client's `X-Request-Id`, when it matched the grammar.
    pub client_id: Option<String>,
    /// Arrival order, the id's number.
    pub seq: u64,
    /// Why the request was refused before any chain ran (`unmatched`, `bad-scope`, `engine-stopped`),
    /// or `client-gone` when the client left before the answer.
    pub rejected: Option<&'static str>,
    /// Unix timestamp in milliseconds.
    pub ts_ms: u64,
    /// Uppercase HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// The matched route template (`GET /users/{id}`); `None` when unmatched.
    pub endpoint: Option<String>,
    /// Response status.
    pub status: u16,
    /// Measured wall-clock milliseconds (not the simulated sum — see the trace header).
    pub ms: u64,
    /// Nodes this request touched, in order.
    pub nodes: Vec<String>,
    /// The node whose chaos failed the request, if any.
    pub failed_node: Option<String>,
    /// The UI state a pin forced on this request (`loading`, `empty`, `error`, `slow` or `success`).
    pub pin: Option<&'static str>,
    /// How it failed when not a plain status (`reset`, `malformed` or `timeout`); `None` for a
    /// status failure or a success. A reset or cut-off body answers 200, so this is how a client tells.
    pub kind: Option<&'static str>,
    /// The request's scenario scope: `None` for the global stack, `Some(key)` for an
    /// `X-Kaanal-Scenario` stack (the names joined by `,`), `Some("")` for an empty header.
    pub scope: Option<String>,
}

/// What the bus carries: a finished request, or a scenario change (S21).
// limitation: `Request` is the large variant (cloned once per subscriber, up to 1024 in the ring);
// `Arc<Event>` is the upgrade path if load shows a cost. Since `Scenario` gained `chaos_key` the
// size gap is under clippy's `large_enum_variant` threshold.
#[derive(Debug, Clone)]
pub enum BusEvent {
    /// A finished mock request (SSE message `request`).
    Request(Event),
    /// A scenario stack change (SSE message `scenario`), or a live chaos lease that ran out.
    Scenario {
        /// `activate`, `deactivate`, `set`, `clear`, `reset`, `expire` (a `ttl` ran out) or `update` (an active scenario's definition changed).
        action: &'static str,
        /// The scenario concerned, when one.
        name: Option<String>,
        /// For a live chaos expiry only: the key whose leased units were cleared, as an object
        /// with the parts it has (`{"node":"db"}`, `{"endpoint":"GET /x"}` or both, the names
        /// of the `PUT /chaos` body); `name` is `None`.
        chaos_key: Option<Value>,
        /// The active stack after the change, bottom to top.
        active: Vec<String>,
    },
}

/// One published chaos state: the stack it was compiled from and the compiled result. Immutable
/// once published; a request clones the `Arc` and uses that one value (S10).
pub struct Effective {
    /// The active scenarios and the live layer, as they were when this was compiled.
    pub stack: ChaosStack,
    /// What the executor reads.
    pub chaos: EffectiveChaos,
    /// The pins in force, folded from the same stack.
    pub pins: EffectivePins,
    /// When each leased activation expires (a copy of [`Control::leases`]).
    pub leases: HashMap<String, tokio::time::Instant>,
    /// When each leased live chaos unit expires (a copy of [`Control::chaos_leases`]).
    pub chaos_leases: crate::chaos_lease::Leases,
}

/// The mutable side of chaos. Behind [`AppState::control`]: the only writer path (S11).
#[derive(Clone, Default)]
pub struct Control {
    /// Every defined scenario.
    pub scenarios: ScenarioSet,
    /// The active scenarios and the live layer.
    pub stack: ChaosStack,
    /// Activations with a `ttl`: scenario name to deadline. Only active names keep one
    /// (`commit` drops the rest).
    pub leases: HashMap<String, tokio::time::Instant>,
    /// Live chaos set with a `ttl`: (key, unit) to deadline. Only units the live layer still
    /// sets keep one (`commit` drops the rest).
    pub chaos_leases: crate::chaos_lease::Leases,
}

/// Shared server state; one `Arc` clones into every handler.
pub struct AppState {
    /// The validated topology graph (immutable after build).
    pub topo: Topology,
    /// Path router: each path maps to its method routes.
    pub paths: matchit::Router<Vec<Route>>,
    /// Sorted `"METHOD /path"` keys, for `/__kaanal/topology` and `status` (synthesized HEAD routes excluded, R6.8).
    pub route_keys: Vec<String>,
    /// Every compiled endpoint, static and synthesized HEAD ones included (the control API checks keys here).
    pub endpoints: Endpoints,
    /// The `scenarios/` directory next to the config, under its canonical directory (it may not exist).
    pub scenarios_dir: PathBuf,
    /// The published chaos state; readers clone the `Arc` and drop the lock ([`AppState::snapshot`]).
    pub chaos: RwLock<Arc<Effective>>,
    /// The scenarios and the stack. Every control write locks it, mutates, and calls
    /// [`AppState::publish`] before unlocking. Never held across an `.await`.
    pub control: Mutex<Control>,
    /// Woken when a scenario or chaos lease is added or replaced, so the lease task re-reads the
    /// earliest deadline of both tables.
    pub lease_wake: tokio::sync::Notify,
    /// Compiled scoped stacks by key (see [`AppState::scoped`]). Lock order: `control`, then this.
    /// [`AppState::publish`] clears it on every publish.
    pub scoped: RwLock<HashMap<String, Arc<crate::scope::Scoped>>>,
    /// Every route's request counter (`POST /reset` zeroes them).
    pub counters: Vec<Arc<AtomicU64>>,
    /// Event bus for the SSE stream (capacity 1024; slow subscribers get `lagged`).
    pub events: broadcast::Sender<BusEvent>,
    /// Bearer token the control API requires.
    pub token: String,
    /// The config's `seed:`: generated bodies use it in both determinism modes (D14).
    pub seed: u64,
    /// Seed of the chain draws: `seed:` under `controlled`, the startup draw under `realistic`.
    pub chain_seed: u64,
    /// The startup draw under `determinism: realistic`, printed by `up` for replay.
    pub drawn_seed: Option<u64>,
    /// The bound port (after the +20 search), used by the Host guard.
    pub port: u16,
    /// Non-fatal findings from loading (ignored schema keywords); shown by `validate` and `up`.
    pub warnings: Vec<String>,
    /// The auth node's token issuer; `None` without an auth node and always in `check`
    /// (`serve` sets it, X5).
    pub auth: Option<crate::auth::Issuer>,
    /// The live replay engine; `None` after `build`, set by `serve` (no runtime in `check`).
    pub engine: Option<crate::actor::EngineHandle>,
    /// Hands out request ids; never reset.
    pub ids: crate::log::IdGen,
    /// The last finished requests. Push only through [`AppState::push_log`].
    pub log: Mutex<crate::log::LogRing>,
    /// The graceful stop: `POST /__kaanal/shutdown` and `BoundServer::shutdown` ask it.
    pub shutdown: crate::shutdown::Shutdown,
    /// The `site:` folder, when the config names one.
    pub site: Option<crate::site::Site>,
    /// The `collection:` items; shared by every scope and user, emptied by `POST /reset`.
    pub collections: crate::collections::Collections,
    /// **Private.** Generation options per node, by [`NodeId`]: the node's `locale`, else the
    /// project's (see [`AppState::options_for_owner`]).
    options: Vec<Options>,
    /// **Private.** The project's options (top-level `locale`, else `en-IN`).
    project_options: Options,
}

/// **Private.** Rejects two paths that differ only in parameter names
/// (`/users/{id}` vs `/users/{uid}`): the router would refuse the second with an
/// unhelpful message. Also reserves the `/__kaanal` namespace (rule R5.4).
fn check_shape(shapes: &mut HashMap<String, String>, path: &str) -> Result<(), String> {
    // A mock under /__kaanal would be served without CORS next to the control API (RULES R5.4).
    if path == "/__kaanal" || path.starts_with("/__kaanal/") {
        return Err(format!("path '{path}' is reserved for the control API"));
    }
    let mut shape = String::new();
    let mut in_param = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_param = true;
                shape.push_str("{}");
            }
            '}' => in_param = false,
            _ if !in_param => shape.push(c),
            _ => {}
        }
    }
    match shapes.get(&shape) {
        Some(first) if first != path => Err(format!(
            "paths '{first}' and '{path}' differ only in parameter names; use the same name in both"
        )),
        Some(_) => Ok(()),
        None => {
            shapes.insert(shape, path.to_string());
            Ok(())
        }
    }
}

/// **Private.** What the spec files define: one [`RouteSpec`] per route, plus its success
/// status and response schema by key, and the paging contract of the paged ones.
struct Specs {
    routes: Vec<RouteSpec>,
    info: BTreeMap<String, (u16, Option<Value>)>,
    paging: BTreeMap<String, Arc<PageSpec>>,
}

/// The topology and its spec routes, read exactly as [`AppState::build`] reads them.
pub(crate) fn spec_routes(
    cfg: &Config,
    base_dir: &Path,
) -> Result<(Topology, Vec<RouteSpec>), Located> {
    let topo = Topology::build(cfg).map_err(|e| e.located(cfg))?;
    let specs = load_specs(&topo, base_dir, &mut HashMap::new(), &mut Vec::new(), None)?;
    Ok((topo, specs.routes))
}

/// **Private.** Reads every node's spec: path shapes, conflicts between services, ignored
/// keywords as warnings. Same checks and messages as Phase One. With `options` (one per node),
/// project dictionary entries that cannot fit a response field are warnings too.
fn load_specs(
    topo: &Topology,
    base_dir: &Path,
    shapes: &mut HashMap<String, String>,
    warnings: &mut Vec<String>,
    options: Option<&[Options]>,
) -> Result<Specs, Located> {
    let mut specs = Specs {
        routes: Vec::new(),
        info: BTreeMap::new(),
        paging: BTreeMap::new(),
    };
    let mut owners: BTreeMap<String, NodeId> = BTreeMap::new();
    for (id, node) in topo.nodes.iter().enumerate() {
        let Some(spec) = &node.spec else { continue };
        let at = keys(&["nodes", node.name.as_str(), "spec"]);
        let file = base_dir.join(spec);
        let src = std::fs::read_to_string(&file)
            .map_err(|e| Located::new(at.clone(), format!("{}: {e}", file.display())))?;
        let api = ApiModel::from_openapi(&src)
            .map_err(|e| Located::new(at.clone(), format!("{}: {e}", file.display())))?;
        warnings.extend(
            api.warnings()
                .into_iter()
                .map(|w| format!("{}: {w}", file.display())),
        );
        if let Some(opts) = options {
            warnings.extend(
                kaanal_data::dictionary_warnings(&api, &opts[id])
                    .into_iter()
                    .map(|w| format!("{}: {w}", file.display())),
            );
        }
        for ep in api.endpoints {
            check_shape(shapes, &ep.path).map_err(|m| Located::new(at.clone(), m))?;
            let key = format!("{} {}", ep.method, ep.path);
            // `compile` takes unique keys; a duplicate is rejected here (P2).
            if let Some(&other) = owners.get(&key) {
                return Err(Located::new(
                    at.clone(),
                    format!(
                        "route {} {} defined by both '{}' and '{}'",
                        ep.method, ep.path, topo.nodes[other].name, node.name
                    ),
                ));
            }
            owners.insert(key.clone(), id);
            specs.routes.push(RouteSpec::inferred(&key, id));
            if let Some(p) = PageSpec::detect(&ep) {
                specs.paging.insert(key.clone(), Arc::new(p));
            }
            specs.info.insert(key, (ep.status, ep.schema));
        }
    }
    Ok(specs)
}

/// **Private.** A data-file error as a [`Located`]: in its file (parse position, else the
/// key path), or at the config value it names (`data:`) when it has no file.
fn data_located(e: DataError) -> Located {
    let path: Vec<PathSeg> =
        e.at.iter()
            .map(|s| {
                s.parse()
                    .map_or_else(|_| PathSeg::Key(s.clone()), PathSeg::Index)
            })
            .collect();
    match e.file {
        Some(f) if e.line.is_some() => Located::parsed(
            f,
            kaanal_core::ConfigError {
                line: e.line,
                col: e.col,
                msg: e.message,
            },
        ),
        Some(f) if e.value => Located::at_value(path, e.message).in_file(f),
        Some(f) => Located::new(path, e.message).in_file(f),
        // no file: the config's own `data:` value
        None => Located::at_value(path, e.message),
    }
}

/// **Private.** The data files (`locales/`, `data.yaml` or `data:`) and the options of every
/// node (its `locale`, else the project's, else `en-IN`), plus the project's own. An unknown
/// locale is placed at the `locale:` that names it (R6.1).
fn load_data(
    cfg: &Config,
    base_dir: &Path,
    topo: &Topology,
    warnings: &mut Vec<String>,
) -> Result<(Vec<Options>, Options), Located> {
    let data = DataSet::load(base_dir, cfg.data.as_deref()).map_err(data_located)?;
    warnings.extend(data.warnings().iter().cloned());
    let project = data
        .options_for(cfg.locale.as_deref().unwrap_or(DEFAULT_LOCALE))
        .map_err(|e| Located::at_value(keys(&["locale"]), format!("locale: {}", e.message)))?;
    let nodes = topo
        .nodes
        .iter()
        .map(|n| match &n.locale {
            None => Ok(project.clone()),
            Some(tag) => data.options_for(tag).map_err(|e| {
                Located::at_value(
                    keys(&["nodes", n.name.as_str(), "locale"]),
                    format!("node '{}': {}", n.name, e.message),
                )
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((nodes, project))
}

/// **Private.** The `site:` folder, if any; a bad one is placed at the `site:` value.
fn load_site(cfg: &Config, base_dir: &Path) -> Result<Option<crate::site::Site>, Located> {
    cfg.site
        .as_deref()
        .map(|rel| {
            crate::site::Site::open(base_dir, rel)
                .map_err(|m| Located::at_value(keys(&["site"]), m))
        })
        .transpose()
}

/// **Private.** Compiles the spec routes with `endpoints:`: the inline map, or the file it
/// names (relative to `base_dir`). A missing file is placed at the `endpoints:` value;
/// errors inside the file carry that file (spec section 8).
fn compile_endpoints(
    cfg: &Config,
    base_dir: &Path,
    topo: &Topology,
    routes: &[RouteSpec],
) -> Result<Endpoints, Located> {
    // P4: a spec route with no entry and no gateway edge would fail in `compile` with no place,
    // but `Topology::build` already rejects a spec node without an edge from the gateway.
    let Some(EndpointsConfig::File(p)) = &cfg.endpoints else {
        return compile(topo, routes, EndpointsInput::inline(cfg));
    };
    let path = base_dir.join(p);
    let src = std::fs::read_to_string(&path)
        .map_err(|e| Located::at_value(keys(&["endpoints"]), format!("{}: {e}", path.display())))?;
    let entries = parse_endpoints(&src).map_err(|e| Located::parsed(path.clone(), e))?;
    compile(
        topo,
        routes,
        EndpointsInput {
            entries: Some(&entries),
            file: Some(&path),
        },
    )
}

/// **Private.** Every scenario: the inline ones, then `scenarios/*.yaml` next to the config
/// (S2, S16). Inline errors are placed in `kaanal.yaml`, file errors in the file. Returns the
/// set and the scenarios directory (under the canonical config directory; it may not exist).
fn load_scenarios(
    cfg: &Config,
    empty_ok: kaanal_core::EmptyOk,
    base_dir: &Path,
    topo: &Topology,
    endpoints: &Endpoints,
    warnings: &mut Vec<String>,
) -> Result<(ScenarioSet, PathBuf), Located> {
    let mut set = ScenarioSet::default();
    for (name, sc) in &cfg.scenarios {
        let at = keys(&["scenarios", name.as_str()]);
        valid_name(name).map_err(|m| Located::new(at.clone(), m))?;
        let (s, w) = resolve_scenario(name, sc, topo, endpoints, empty_ok)
            .map_err(|e| e.located(at.clone()))?;
        warnings.extend(w);
        set.insert(s, Source::Inline)
            .map_err(|m| Located::new(at, m))?;
    }
    // Canonicalized once, so a later `cd` cannot move it (S16). A symlinked `scenarios/` is
    // followed here, once; its target is what `save` writes through. Absent: the joined path.
    let dir = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf())
        .join("scenarios");
    let dir = dir.canonicalize().unwrap_or(dir);
    let (found, found_warnings) = scenarios::discover(&dir)?;
    warnings.extend(found_warnings);
    for (name, sc, file) in found {
        let (s, w) = resolve_scenario(&name, &sc, topo, endpoints, empty_ok)
            .map_err(|e| e.located(Vec::new()).in_file(file.clone()))?;
        warnings.extend(w);
        set.insert(s, Source::File)
            .map_err(|m| Located::new(Vec::new(), m).in_file(file))?;
    }
    Ok((set, dir))
}

impl AppState {
    /// The published chaos state. One `Arc` clone; the lock is gone on return.
    pub fn snapshot(&self) -> Arc<Effective> {
        Arc::clone(&self.chaos.read().expect("chaos lock poisoned"))
    }

    /// Records a finished request and returns its `done`. The lock is short and never held across
    /// an `.await`; a poisoned lock is used as is (the ring holds plain data).
    pub fn push_log(&self, e: crate::log::LogEntry) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(e)
    }

    /// Compiles `c` and swaps the result in; returns what was published. Taking the guard
    /// proves the caller holds the [`Control`] lock, so publishes happen in write order (S11).
    ///
    /// # Errors
    /// The compiled result breaks a chaos limit (a fault rate outside 0..=1 or a status outside
    /// 500-504). Each layer is validated on entry and latency is clamped by the compile, so
    /// this cannot happen today; it guards the invariant. Nothing is swapped.
    pub fn publish(&self, c: &MutexGuard<'_, Control>) -> Result<Arc<Effective>, String> {
        let chaos = c.stack.compile(&c.scenarios);
        for (_, r) in chaos.entries() {
            for f in r.faults() {
                // The union folds in floats, so allow a hair of rounding above 1.
                if !(0.0..=1.0 + 1e-9).contains(&f.upto) || !(500..=504).contains(&f.status) {
                    return Err(
                        "the combined chaos is out of range (error_rate 0-1, error_status 500-504)"
                            .into(),
                    );
                }
            }
        }
        let next = Arc::new(Effective {
            stack: c.stack.clone(),
            chaos,
            pins: c.stack.compile_pins(&c.scenarios),
            leases: c.leases.clone(),
            chaos_leases: c.chaos_leases.clone(),
        });
        *self.chaos.write().expect("chaos lock poisoned") = Arc::clone(&next);
        // Any definition or stack change may alter a scoped stack; the caller holds `control`.
        self.scoped.write().expect("scoped lock poisoned").clear();
        Ok(next)
    }

    /// Runs the full load pipeline: topology → spec files → endpoints (inline or file) →
    /// compiled routes → matchit router.
    ///
    /// Spec paths are resolved against `base_dir` (the config file's directory).
    /// Routes are inserted in deterministic (`BTreeMap`) order so a conflict always
    /// names the same path.
    ///
    /// # Errors
    /// Returns a [`Located`] error (path into `kaanal.yaml` + message, with file paths
    /// where relevant) for unreadable or invalid specs or endpoints file, route conflicts,
    /// any `endpoints:` error from `compile` (placed in the endpoints file when it comes
    /// from there), or any topology error. Nothing is served half-configured (rule R6.1).
    #[expect(
        clippy::too_many_lines,
        reason = "one linear load pipeline; two lines over after the site: step"
    )]
    pub fn build(
        cfg: &Config,
        base_dir: &Path,
        token: String,
        port: u16,
    ) -> Result<AppState, Located> {
        let topo = Topology::build(cfg).map_err(|e| e.located(cfg))?;
        // `/users/{id}` -> `/users/{}`; the router cannot hold two spellings of one shape.
        let mut shapes: HashMap<String, String> = HashMap::new();
        let mut warnings = Vec::new();
        // P9: a login mock with no accounts answers 401 to everyone; say so up front.
        for (name, n) in &cfg.nodes {
            if let NodeConfig::Auth(a) = n
                && a.users.is_empty()
            {
                warnings.push(format!(
                    "node '{name}': no users, so every POST /login answers 401 (add users: [{{ username, password }}])"
                ));
            }
        }

        let (options, project_options) = load_data(cfg, base_dir, &topo, &mut warnings)?;
        let site = load_site(cfg, base_dir)?;
        let specs = load_specs(&topo, base_dir, &mut shapes, &mut warnings, Some(&options))?;
        let endpoints = compile_endpoints(cfg, base_dir, &topo, &specs.routes)?;
        warnings.extend(endpoints.warnings.iter().cloned());
        // The seed only changes generated values, not whether an empty form exists.
        let ok = crate::pins::empty_ok(&specs.info, cfg.seed);
        let (scenarios, scenarios_dir) =
            load_scenarios(cfg, &ok, base_dir, &topo, &endpoints, &mut warnings)?;
        // BTreeMap and `Endpoints::iter` (sorted by key): a conflict always names the same path.
        let mut by_path: BTreeMap<String, Vec<Route>> = BTreeMap::new();
        let mut counters = Vec::new();
        for ep in endpoints.iter() {
            let (method, path) = ep.key.split_once(' ').unwrap_or((ep.key.as_str(), ""));
            if ep.owner.is_none() {
                // A static route: the spec loop never saw its path.
                check_shape(&mut shapes, path).map_err(|m| ep.located(m))?;
            }
            // HEAD without a spec entry of its own answers like GET (R6.8).
            let spec = specs.info.get(&ep.key).or_else(|| {
                if method == "HEAD" {
                    specs.info.get(&format!("GET {path}"))
                } else {
                    None
                }
            });
            let (status, schema) = spec.cloned().unwrap_or((200, None));
            // A HEAD takes its GET's paging only when the spec has no HEAD entry of its own (a
            // declared HEAD is paged by its own parameters, which `detect` never pages); a route
            // that does not generate its body is never paged.
            let generated =
                ep.response.is_none() && ep.collection.is_none() && ep.responder.is_none();
            let get = format!("GET {path}");
            let follows_get = method == "HEAD" && !specs.info.contains_key(&ep.key);
            let paging = specs
                .paging
                .get(&ep.key)
                .or_else(|| specs.paging.get(&get).filter(|_| follows_get))
                .filter(|_| generated)
                .cloned();
            // Such a HEAD with an entry of its own but no `total:` reports its GET's total.
            let total = ep.total.or_else(|| {
                endpoints
                    .get(&get)
                    .filter(|_| follows_get)
                    .and_then(|g| g.total)
            });
            if ep.total.is_some() && paging.is_none() && !ep.synthesized {
                warnings.push(format!(
                    "endpoint '{}': `total` has no effect: the route is not paged (no paging query parameter, or no list in its response, or the route does not generate its body: override, collection or built-in responder)",
                    ep.key
                ));
            }
            let counter = Arc::new(AtomicU64::new(0));
            counters.push(Arc::clone(&counter));
            by_path.entry(path.to_string()).or_default().push(Route {
                method: method.to_string(),
                key: ep.key.clone(),
                endpoint: Arc::clone(ep),
                status,
                schema,
                counter,
                paging,
                total,
            });
        }
        let route_keys: Vec<String> = endpoints
            .iter()
            .filter(|e| !e.synthesized)
            .map(|e| e.key.clone())
            .collect();
        let mut paths = matchit::Router::new();
        for (p, routes) in by_path {
            paths
                .insert(p.clone(), routes)
                .map_err(|e| Located::new(Vec::new(), format!("route '{p}': {e}")))?;
        }
        let drawn_seed = match cfg.seed_source() {
            SeedSource::Fixed(_) => None,
            // R2.3: entropy is the server's job, drawn once here; no RNG state is kept.
            SeedSource::Entropy => Some(rand::random::<u64>()),
        };
        let (events, _) = broadcast::channel(1024);
        let ids = crate::log::IdGen::new();
        let log = Mutex::new(crate::log::LogRing::new(ids.boot()));
        Ok(AppState {
            topo,
            paths,
            route_keys,
            endpoints,
            scenarios_dir,
            chaos: RwLock::new(Arc::new(Effective {
                stack: ChaosStack::default(),
                chaos: EffectiveChaos::default(),
                pins: EffectivePins::default(),
                leases: HashMap::new(),
                chaos_leases: HashMap::new(),
            })),
            control: Mutex::new(Control {
                scenarios,
                stack: ChaosStack::default(),
                leases: HashMap::new(),
                chaos_leases: HashMap::new(),
            }),
            lease_wake: tokio::sync::Notify::new(),
            scoped: RwLock::new(HashMap::new()),
            counters,
            events,
            token,
            seed: cfg.seed,
            chain_seed: drawn_seed.unwrap_or(cfg.seed),
            drawn_seed,
            port,
            warnings,
            auth: None,
            engine: None,
            site,
            collections: crate::collections::Collections::new(),
            ids,
            log,
            shutdown: crate::shutdown::Shutdown::new(),
            options,
            project_options,
        })
    }

    /// The generation options of a route owned by `owner`; the project's for a route without
    /// an owner (static routes have no schema, so this is a fallback).
    pub fn options_for_owner(&self, owner: Option<NodeId>) -> &Options {
        owner.map_or(&self.project_options, |id| &self.options[id])
    }
}
