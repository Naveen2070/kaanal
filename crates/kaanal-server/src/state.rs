use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;

use kaanal_core::{ChaosState, Config, NodeId, Op, Topology};
use kaanal_data::ApiModel;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

pub enum Body {
    Generated { schema: Option<Value> },
    Static(Value),
}

pub struct Route {
    pub method: String,
    /// "GET /users/{id}" — chain RNG key and display name.
    pub key: String,
    /// Service owning the route; `None` = static-only route answered by the gateway.
    pub owner: Option<NodeId>,
    pub op: Op,
    pub status: u16,
    pub body: Body,
    /// Requests seen so far; becomes `ChainRequest.n`.
    pub counter: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub ts_ms: u64,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub ms: u64,
    /// Nodes this request touched, in order.
    pub nodes: Vec<String>,
    pub failed_node: Option<String>,
}

pub struct AppState {
    pub topo: Topology,
    pub paths: matchit::Router<Vec<Route>>,
    pub route_keys: Vec<String>,
    pub chaos: RwLock<ChaosState>,
    pub events: broadcast::Sender<Event>,
    pub token: String,
    pub seed: u64,
    pub port: u16,
    /// Non-fatal findings from loading (ignored schema keywords); shown by `validate` and `up`.
    pub warnings: Vec<String>,
}

const METHODS: [&str; 7] = ["GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Reject two paths that differ only in parameter names (`/users/{id}` vs `/users/{uid}`):
/// the router would refuse the second with an unhelpful message.
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

impl AppState {
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
            let (m, p) = spec
                .split_once(' ')
                .ok_or_else(|| format!("routes: '{spec}' must look like 'GET /path'"))?;
            let (m, p) = (m.to_uppercase(), p.trim().to_string());
            if !(100..=599).contains(&ov.status) {
                return Err(format!(
                    "routes: '{spec}': status {} must be between 100 and 599",
                    ov.status
                ));
            }
            if !METHODS.contains(&m.as_str()) {
                return Err(format!("routes: '{spec}': unknown method '{m}'"));
            }
            if !p.starts_with('/') {
                return Err(format!("routes: '{spec}': path must start with '/'"));
            }
            check_shape(&mut shapes, &p)?;
            let routes = by_path.entry(p.clone()).or_default();
            match routes.iter_mut().find(|r| r.method == m) {
                Some(r) => {
                    r.status = ov.status;
                    r.body = Body::Static(ov.body.clone());
                }
                None => routes.push(Route {
                    key: format!("{m} {p}"),
                    op: Op::infer(&m, &p),
                    method: m,
                    owner: None,
                    status: ov.status,
                    body: Body::Static(ov.body.clone()),
                    counter: AtomicU64::new(0),
                }),
            }
        }

        let mut route_keys: Vec<String> =
            by_path.values().flatten().map(|r| r.key.clone()).collect();
        route_keys.sort();
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
