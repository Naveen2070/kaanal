//! [`AppState`]: everything the running server knows, built once at startup.
//!
//! [`AppState::build`] is the full load pipeline shared by `serve` and `check`
//! (`kaanal validate`): topology → `OpenAPI` specs → endpoints (inline or the endpoints
//! file) compiled by `kaanal_core::chain::compile` → route table. Every problem is a
//! fatal error with a precise message, or — for ignorable schema keywords — a collected
//! warning.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::config::{EndpointsConfig, SeedSource, parse_endpoints};
use kaanal_core::diagnostics::{Located, keys};
use kaanal_core::{ChaosState, Config, Endpoint, Endpoints, NodeId, Topology};
use kaanal_data::ApiModel;
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
    pub counter: AtomicU64,
}

/// One finished request, broadcast to `/__kaanal/events` subscribers.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Unix timestamp in milliseconds.
    pub ts_ms: u64,
    /// Uppercase HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Response status.
    pub status: u16,
    /// Measured wall-clock milliseconds (not the simulated sum — see the trace header).
    pub ms: u64,
    /// Nodes this request touched, in order.
    pub nodes: Vec<String>,
    /// The node whose chaos failed the request, if any.
    pub failed_node: Option<String>,
}

/// Shared server state; one `Arc` clones into every handler.
pub struct AppState {
    /// The validated topology graph (immutable after build).
    pub topo: Topology,
    /// Path router: each path maps to its method routes.
    pub paths: matchit::Router<Vec<Route>>,
    /// Sorted `"METHOD /path"` keys, for `/__kaanal/topology` and `status` (synthesized HEAD routes excluded, R6.8).
    pub route_keys: Vec<String>,
    /// Live chaos settings; written by the control API, read (briefly) per request.
    pub chaos: RwLock<ChaosState>,
    /// Request-event bus for the SSE stream (capacity 1024; slow subscribers get `lagged`).
    pub events: broadcast::Sender<Event>,
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
    /// Non-fatal findings from loading (ignored schema keywords, nodes with `concurrency`); shown by `validate` and `up`.
    pub warnings: Vec<String>,
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
/// status and response schema by key.
struct Specs {
    routes: Vec<RouteSpec>,
    info: BTreeMap<String, (u16, Option<Value>)>,
}

/// The topology and its spec routes, read exactly as [`AppState::build`] reads them.
pub(crate) fn spec_routes(
    cfg: &Config,
    base_dir: &Path,
) -> Result<(Topology, Vec<RouteSpec>), Located> {
    let topo = Topology::build(cfg).map_err(|e| e.located(cfg))?;
    let specs = load_specs(&topo, base_dir, &mut HashMap::new(), &mut Vec::new())?;
    Ok((topo, specs.routes))
}

/// **Private.** Reads every node's spec: path shapes, conflicts between services, ignored
/// keywords as warnings. Same checks and messages as Phase One.
fn load_specs(
    topo: &Topology,
    base_dir: &Path,
    shapes: &mut HashMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<Specs, Located> {
    let mut specs = Specs {
        routes: Vec::new(),
        info: BTreeMap::new(),
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
            specs.info.insert(key, (ep.status, ep.schema));
        }
    }
    Ok(specs)
}

/// **Private.** Compiles the spec routes with `endpoints:`: the inline map, or the file it
/// names (relative to `base_dir`). A missing file is placed at the `endpoints:` value;
/// errors inside the file carry that file (spec §8).
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

impl AppState {
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
        // Live mode has no queues yet; say so instead of silently ignoring the setting.
        for n in topo.nodes.iter().filter(|n| n.concurrency.is_some()) {
            warnings.push(format!(
                "node '{}': concurrency is honoured by the simulator only (live mode enforces it from M1)",
                n.name
            ));
        }

        let specs = load_specs(&topo, base_dir, &mut shapes, &mut warnings)?;
        let endpoints = compile_endpoints(cfg, base_dir, &topo, &specs.routes)?;
        warnings.extend(endpoints.warnings.iter().cloned());
        // BTreeMap and `Endpoints::iter` (sorted by key): a conflict always names the same path.
        let mut by_path: BTreeMap<String, Vec<Route>> = BTreeMap::new();
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
            by_path.entry(path.to_string()).or_default().push(Route {
                method: method.to_string(),
                key: ep.key.clone(),
                endpoint: Arc::clone(ep),
                status,
                schema,
                counter: AtomicU64::new(0),
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
        Ok(AppState {
            topo,
            paths,
            route_keys,
            chaos: RwLock::new(ChaosState::default()),
            events,
            token,
            seed: cfg.seed,
            chain_seed: drawn_seed.unwrap_or(cfg.seed),
            drawn_seed,
            port,
            warnings,
        })
    }
}
