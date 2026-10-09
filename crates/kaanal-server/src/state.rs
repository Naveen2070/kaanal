//! [`AppState`]: everything the running server knows, built once at startup.
//!
//! [`AppState::build`] is the full load pipeline shared by `serve` and `check`
//! (`kaanal validate`): topology → `OpenAPI` specs → route table → overrides. Every
//! problem is a fatal error with a precise message, or — for ignorable schema
//! keywords — a collected warning.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;

use kaanal_core::config::RouteOverride;
use kaanal_core::{ChaosState, Config, NodeId, Op, Topology};
use kaanal_data::ApiModel;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

/// Where a route's response body comes from.
#[derive(Clone)]
pub enum Body {
    /// Generated from an `OpenAPI` response schema (`None` = empty body).
    Generated {
        /// The inlined JSON schema, if the spec declares one.
        schema: Option<Value>,
    },
    /// A static `routes:` override, returned verbatim.
    Static(Value),
}

/// One route: a `method + path` pair and how to answer it.
pub struct Route {
    /// Uppercase HTTP method.
    pub method: String,
    /// "GET /users/{id}" — chain RNG key and display name.
    pub key: String,
    /// Service or monolith owning the route; `None` = static-only route answered by the gateway.
    pub owner: Option<NodeId>,
    /// The database operation this endpoint performs (inferred from method + path).
    pub op: Op,
    /// HTTP status of a successful response.
    pub status: u16,
    /// Generated schema or static body.
    pub body: Body,
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
    /// Sorted `"METHOD /path"` keys, for `/__kaanal/topology` and `status` (synthesised HEAD routes excluded).
    pub route_keys: Vec<String>,
    /// Live chaos settings; written by the control API, read (briefly) per request.
    pub chaos: RwLock<ChaosState>,
    /// Request-event bus for the SSE stream (capacity 1024; slow subscribers get `lagged`).
    pub events: broadcast::Sender<Event>,
    /// Bearer token the control API requires.
    pub token: String,
    /// Master RNG seed from the config.
    pub seed: u64,
    /// The bound port (after the +20 search), used by the Host guard.
    pub port: u16,
    /// Non-fatal findings from loading (ignored schema keywords, nodes with `concurrency`); shown by `validate` and `up`.
    pub warnings: Vec<String>,
}

/// **Private.** Uppercase methods accepted in `routes:` override keys.
const METHODS: [&str; 7] = ["GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS"];

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

/// **Private.** R6.8: a path with GET but no HEAD gets a `HEAD /p` route that answers like
/// GET (owner, op, status, body, `routes:` override) under its own key and counter, so
/// HEAD probes never shift GET's chain sequence.
fn add_head_routes(by_path: &mut BTreeMap<String, Vec<Route>>) {
    for (p, routes) in by_path {
        if routes.iter().any(|r| r.method == "HEAD") {
            continue;
        }
        if let Some(get) = routes.iter().find(|r| r.method == "GET") {
            let head = Route {
                method: "HEAD".into(),
                key: format!("HEAD {p}"),
                owner: get.owner,
                op: get.op,
                status: get.status,
                body: get.body.clone(),
                counter: AtomicU64::new(0),
            };
            routes.push(head);
        }
    }
}

/// **Private.** Applies one `routes:` entry: validates the key and status, then either
/// overrides the route a service owns or adds a static-only route. Without a gateway
/// nothing answers a route no service owns, so that is an error (R1.23).
fn apply_override(
    by_path: &mut BTreeMap<String, Vec<Route>>,
    shapes: &mut HashMap<String, String>,
    spec: &str,
    ov: &RouteOverride,
    has_gateway: bool,
) -> Result<(), String> {
    let (m, p) = spec
        .split_once(' ')
        .ok_or_else(|| format!("routes: '{spec}' must look like 'GET /path'"))?;
    let (m, p) = (m.to_uppercase(), p.trim().to_string());
    // 1xx is not a final response: hyper would turn it into its own 500 (R6.2).
    if !(200..=599).contains(&ov.status) {
        return Err(format!(
            "routes: '{spec}': status {} must be between 200 and 599",
            ov.status
        ));
    }
    if !METHODS.contains(&m.as_str()) {
        return Err(format!("routes: '{spec}': unknown method '{m}'"));
    }
    if !p.starts_with('/') {
        return Err(format!("routes: '{spec}': path must start with '/'"));
    }
    check_shape(shapes, &p)?;
    let routes = by_path.entry(p.clone()).or_default();
    if let Some(r) = routes.iter_mut().find(|r| r.method == m) {
        r.status = ov.status;
        r.body = Body::Static(ov.body.clone());
        return Ok(());
    }
    // R6.8: a HEAD override of a path the service owns keeps GET's owner and op, so the
    // service hop is not lost (and it is not a gateway-only static route).
    if m == "HEAD"
        && let Some(get) = routes.iter().find(|r| r.method == "GET")
    {
        let head = Route {
            key: format!("HEAD {p}"),
            op: get.op,
            method: m,
            owner: get.owner,
            status: ov.status,
            body: Body::Static(ov.body.clone()),
            counter: AtomicU64::new(0),
        };
        routes.push(head);
        return Ok(());
    }
    if !has_gateway {
        return Err(format!(
            "routes: '{spec}': static route needs a gateway or a service that owns it"
        ));
    }
    routes.push(Route {
        key: format!("{m} {p}"),
        op: Op::infer(&m, &p),
        method: m,
        owner: None,
        status: ov.status,
        body: Body::Static(ov.body.clone()),
        counter: AtomicU64::new(0),
    });
    Ok(())
}

impl AppState {
    /// Runs the full load pipeline: topology → spec files → spec routes → `routes:`
    /// overrides → matchit router.
    ///
    /// Spec paths are resolved against `base_dir` (the config file's directory).
    /// Routes are inserted in deterministic (`BTreeMap`) order so a conflict always
    /// names the same path.
    ///
    /// # Errors
    /// Returns a message (with file paths where relevant) for unreadable or invalid
    /// specs, route conflicts between services, malformed override keys, or any
    /// topology error. Nothing is served half-configured (rule R6.1).
    pub fn build(
        cfg: &Config,
        base_dir: &Path,
        token: String,
        port: u16,
    ) -> Result<AppState, String> {
        let topo = Topology::build(cfg).map_err(|e| e.to_string())?;
        // BTreeMap: deterministic order, so a router conflict always names the same path.
        let mut by_path: BTreeMap<String, Vec<Route>> = BTreeMap::new();
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

        for (id, node) in topo.nodes.iter().enumerate() {
            let Some(spec) = &node.spec else { continue };
            let file = base_dir.join(spec);
            let src =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let api =
                ApiModel::from_openapi(&src).map_err(|e| format!("{}: {e}", file.display()))?;
            warnings.extend(
                api.warnings()
                    .into_iter()
                    .map(|w| format!("{}: {w}", file.display())),
            );
            for ep in api.endpoints {
                check_shape(&mut shapes, &ep.path)?;
                let routes = by_path.entry(ep.path.clone()).or_default();
                if let Some(other) = routes.iter().find(|r| r.method == ep.method) {
                    let owner = other.owner.map_or("?", |o| topo.nodes[o].name.as_str());
                    return Err(format!(
                        "route {} {} defined by both '{owner}' and '{}'",
                        ep.method, ep.path, node.name
                    ));
                }
                routes.push(Route {
                    key: format!("{} {}", ep.method, ep.path),
                    op: Op::infer(&ep.method, &ep.path),
                    method: ep.method,
                    owner: Some(id),
                    status: ep.status,
                    body: Body::Generated { schema: ep.schema },
                    counter: AtomicU64::new(0),
                });
            }
        }

        for (spec, ov) in &cfg.routes {
            apply_override(&mut by_path, &mut shapes, spec, ov, topo.gateway.is_some())?;
        }

        let mut route_keys: Vec<String> =
            by_path.values().flatten().map(|r| r.key.clone()).collect();
        route_keys.sort();
        // After `route_keys`, so synthesised HEAD routes are not listed (R6.8).
        add_head_routes(&mut by_path);
        let mut paths = matchit::Router::new();
        for (p, routes) in by_path {
            paths
                .insert(p.clone(), routes)
                .map_err(|e| format!("route '{p}': {e}"))?;
        }
        let (events, _) = broadcast::channel(1024);
        Ok(AppState {
            topo,
            paths,
            route_keys,
            chaos: RwLock::new(ChaosState::default()),
            events,
            token,
            seed: cfg.seed,
            port,
            warnings,
        })
    }
}
