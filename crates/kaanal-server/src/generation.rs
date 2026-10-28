//! [`Generation`]: everything built from one config (topology, routes, endpoints, data options,
//! engine), the part of the server a later Apply can swap. [`Generation::build`] is the full load
//! pipeline shared by `serve` and `check`: topology → `OpenAPI` specs → endpoints (inline or the
//! endpoints file) → route table. Every problem is a fatal error with a precise message, or, for
//! ignorable schema keywords, a collected warning. Moved out of `state.rs` (R3.8); the only change
//! to the pipeline is that the realistic-mode `drawn_seed` is now a parameter.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::config::{EndpointsConfig, NodeConfig, parse_endpoints};
use kaanal_core::diagnostics::{Located, PathSeg, keys};
use kaanal_core::{
    Config, Endpoints, NodeId, ScenarioSet, Source, Topology, resolve_scenario, valid_name,
};
use kaanal_data::{ApiModel, DEFAULT_LOCALE, DataError, DataSet, Options, PageSpec};
use serde_json::Value;

use crate::paths::{read_text, resolve, shown};
use crate::scenarios;
use crate::state::Route;

pub use crate::paths::Trust;

/// The extractor for a handler that needs the system: the generation in force, taken once at the
/// start of the request. Answers 409 while a blank start has no system yet.
pub struct Gen(pub Arc<Generation>);

impl axum::extract::FromRequestParts<Arc<crate::state::AppState>> for Gen {
    type Rejection = (axum::http::StatusCode, axum::Json<serde_json::Value>);

    fn from_request_parts(
        _: &mut axum::http::request::Parts,
        st: &Arc<crate::state::AppState>,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(st.generation().map(Gen).ok_or((
            axum::http::StatusCode::CONFLICT,
            axum::Json(serde_json::json!({ "error": "no system applied yet; apply a draft" })),
        )))
    }
}

/// A scenario file the build left out: it names something the topology does not have.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Flagged {
    /// The scenario file.
    pub file: String,
    /// The scenario's name (its file stem).
    pub scenario: String,
    /// What did not resolve, as the loader words it.
    pub message: String,
    /// The file as the loader found it (not shown to clients: `file` is theirs).
    #[serde(skip)]
    pub path: PathBuf,
}

/// What one config builds: the topology, the route table, the compiled endpoints and the data
/// options. Immutable once built.
pub struct Generation {
    /// Which generation this is: 1 for the first, counting up with every Apply.
    pub id: u64,
    /// The validated topology graph.
    pub topo: Topology,
    /// Path router: each path maps to its method routes.
    pub paths: matchit::Router<Vec<Route>>,
    /// Sorted `"METHOD /path"` keys, for `/__kaanal/topology` and `status` (synthesized HEAD routes excluded, R6.8).
    pub route_keys: Vec<String>,
    /// Every compiled endpoint, static and synthesized HEAD ones included.
    pub endpoints: Endpoints,
    /// Every route's request counter (`POST /reset` zeroes them).
    pub counters: Vec<Arc<AtomicU64>>,
    /// The `scenarios/` directory next to the config, under its canonical directory (it may not exist).
    pub scenarios_dir: PathBuf,
    /// The `site:` folder, when the config names one.
    pub site: Option<crate::site::Site>,
    /// Generation options per node, by [`NodeId`]: the node's `locale`, else the project's.
    options: Vec<Options>,
    /// The project's options (top-level `locale`, else `en-IN`).
    project_options: Options,
    /// Non-fatal findings from loading (ignored schema keywords); shown by `validate` and `up`.
    pub warnings: Vec<String>,
    /// The config's `seed:`: generated bodies use it in both determinism modes (D14).
    pub seed: u64,
    /// Seed of the chain draws: `seed:` under `controlled`, the drawn one under `realistic`.
    pub chain_seed: u64,
    /// The config's named traffic profiles (`traffic:`), for the simulator.
    pub traffic: indexmap::IndexMap<String, kaanal_core::TrafficConfig>,
    /// Scenario files left out because they do not resolve against this topology. Always empty
    /// for a strict build (startup, `check`), where such a file is an error.
    pub flagged: Vec<Flagged>,
    /// The auth node's token issuer; `None` without an auth node and always in `check`
    /// (`serve` sets it, X5).
    pub auth: Option<crate::auth::Issuer>,
    /// The live replay engine; `None` after `build`, set by `serve` (no runtime in `check`).
    pub engine: Option<crate::actor::EngineHandle>,
}

impl Generation {
    /// The generation options of a route owned by `owner`; the project's for a route without
    /// an owner (static routes have no schema, so this is a fallback).
    pub fn options_for_owner(&self, owner: Option<NodeId>) -> &Options {
        owner.map_or(&self.project_options, |id| &self.options[id])
    }

    /// Runs the full load pipeline: topology → spec files → endpoints (inline or file) →
    /// compiled routes → matchit router. Spec paths are resolved against `base_dir` (the config
    /// file's directory). Routes are inserted in deterministic (`BTreeMap`) order so a conflict
    /// always names the same path. `drawn_seed` is the realistic-mode startup draw, passed in so it
    /// stays the same across generations. With [`Trust::Api`] (a draft), the paths in the config are
    /// held to the project folder, and a scenario *file* that does not resolve against the new
    /// topology (it names a node the draft removed) is left out and listed in `flagged` instead of
    /// failing the build; inline scenarios are part of the draft and stay strict. Also returns the scenarios the config and `scenarios/`
    /// define, resolved against this topology.
    ///
    /// # Errors
    /// A [`Located`] error (path into `kaanal.yaml` + message, with file paths where relevant) for
    /// unreadable or invalid specs or endpoints file, route conflicts, any `endpoints:` error from
    /// `compile`, or any topology error. Nothing is served half-configured (rule R6.1).
    #[expect(
        clippy::too_many_lines,
        reason = "one linear load pipeline; one line over after the generation id"
    )]
    pub fn build(
        cfg: &Config,
        base_dir: &Path,
        drawn_seed: Option<u64>,
        trust: Trust,
    ) -> Result<(Generation, ScenarioSet), Located> {
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

        let (options, project_options) = load_data(cfg, base_dir, &topo, &mut warnings, trust)?;
        let site = load_site(cfg, base_dir)?;
        let specs = load_specs(
            &topo,
            base_dir,
            &mut shapes,
            &mut warnings,
            Some(&options),
            trust,
        )?;
        let endpoints = compile_endpoints(cfg, base_dir, &topo, &specs.routes, trust)?;
        warnings.extend(endpoints.warnings.iter().cloned());
        // The seed only changes generated values, not whether an empty form exists.
        let ok = crate::pins::empty_ok(&specs.info, cfg.seed);
        let (scenarios, scenarios_dir, flagged) = load_scenarios(
            cfg,
            &ok,
            base_dir,
            &topo,
            &endpoints,
            &mut warnings,
            trust == Trust::Api,
        )?;
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
        for (name, profile) in &cfg.traffic {
            for j in &profile.journeys {
                let known = kaanal_core::endpoint_key(&j.endpoint)
                    .is_ok_and(|k| endpoints.get(&k).is_some());
                if !known {
                    warnings.push(format!(
                        "traffic profile '{name}': endpoint '{}' is not a route of this system, so a run of it will be refused",
                        j.endpoint
                    ));
                }
            }
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
        Ok((
            Generation {
                id: 1,
                topo,
                paths,
                route_keys,
                endpoints,
                counters,
                scenarios_dir,
                site,
                options,
                project_options,
                warnings,
                seed: cfg.seed,
                chain_seed: drawn_seed.unwrap_or(cfg.seed),
                traffic: cfg.traffic.clone(),
                flagged,
                auth: None,
                engine: None,
            },
            scenarios,
        ))
    }
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

/// The topology and its spec routes, read exactly as [`Generation::build`] reads them.
pub(crate) fn spec_routes(
    cfg: &Config,
    base_dir: &Path,
) -> Result<(Topology, Vec<RouteSpec>), Located> {
    let topo = Topology::build(cfg).map_err(|e| e.located(cfg))?;
    let specs = load_specs(
        &topo,
        base_dir,
        &mut HashMap::new(),
        &mut Vec::new(),
        None,
        Trust::Owner,
    )?;
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
    trust: Trust,
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
        let (file, api) = read_api(spec, base_dir, &at, trust)?;
        let label = shown(base_dir, &file, trust);
        warnings.extend(api.warnings().into_iter().map(|w| format!("{label}: {w}")));
        if let Some(opts) = options {
            warnings.extend(
                kaanal_data::dictionary_warnings(&api, &opts[id])
                    .into_iter()
                    .map(|w| format!("{label}: {w}")),
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

/// **Private.** Reads and parses one node's spec file; an error is placed at `at` (the
/// node's `spec:` key) and names the file.
pub(crate) fn read_api(
    spec: &str,
    base_dir: &Path,
    at: &[PathSeg],
    trust: Trust,
) -> Result<(PathBuf, ApiModel), Located> {
    let file = resolve(base_dir, spec, trust, at)?;
    let label = shown(base_dir, &file, trust);
    let src = read_text(&file).map_err(|e| Located::new(at.to_vec(), format!("{label}: {e}")))?;
    let api = ApiModel::from_openapi(&src).map_err(|e| {
        // a parse error can quote the file's text; the API must not echo a file it was only pointed at
        let msg = if trust == Trust::Api {
            "not a valid OpenAPI document".to_string()
        } else {
            e.clone()
        };
        Located::new(at.to_vec(), format!("{label}: {msg}"))
    })?;
    Ok((file, api))
}

/// **Private.** A data-file error as a [`Located`]: in its file (parse position, else the
/// key path), or at the config value it names (`data:`) when it has no file.
fn data_located(e: DataError, trust: Trust) -> Located {
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
                // a parse error can quote the file's text (see `read_api`)
                msg: if trust == Trust::Api {
                    "not a valid data file".to_string()
                } else {
                    e.message
                },
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
pub(crate) fn load_data(
    cfg: &Config,
    base_dir: &Path,
    topo: &Topology,
    warnings: &mut Vec<String>,
    trust: Trust,
) -> Result<(Vec<Options>, Options), Located> {
    if let Some(rel) = cfg.data.as_deref() {
        resolve(base_dir, rel, trust, &keys(&["data"]))?;
    }
    let data = DataSet::load(base_dir, cfg.data.as_deref()).map_err(|e| data_located(e, trust))?;
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
    trust: Trust,
) -> Result<Endpoints, Located> {
    // P4: a spec route with no entry and no gateway edge would fail in `compile` with no place,
    // but `Topology::build` already rejects a spec node without an edge from the gateway.
    let Some(EndpointsConfig::File(p)) = &cfg.endpoints else {
        return compile(topo, routes, EndpointsInput::inline(cfg));
    };
    let at = keys(&["endpoints"]);
    let path = resolve(base_dir, p, trust, &at)?;
    let label = shown(base_dir, &path, trust);
    let src =
        read_text(&path).map_err(|e| Located::at_value(at.clone(), format!("{label}: {e}")))?;
    let entries = parse_endpoints(&src).map_err(|mut e| {
        // a parse error can quote the file's text (see `read_api`)
        if trust == Trust::Api {
            e.msg = "not a valid endpoints file".to_string();
        }
        Located::parsed(path.clone(), e)
    })?;
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
    lenient: bool,
) -> Result<(ScenarioSet, PathBuf, Vec<Flagged>), Located> {
    let mut set = ScenarioSet::default();
    let mut flagged = Vec::new();
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
        let resolved = resolve_scenario(&name, &sc, topo, endpoints, empty_ok);
        let (s, w) = match resolved {
            Ok(ok) => ok,
            Err(e) if lenient => {
                flagged.push(Flagged {
                    file: file.display().to_string(),
                    scenario: name,
                    message: e.located(Vec::new()).msg,
                    path: file.clone(),
                });
                continue;
            }
            Err(e) => return Err(e.located(Vec::new()).in_file(file)),
        };
        warnings.extend(w);
        set.insert(s, Source::File)
            .map_err(|m| Located::new(Vec::new(), m).in_file(file))?;
    }
    Ok((set, dir, flagged))
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "version: 1
seed: 42
nodes:
  gw: { type: gateway }
  app: { type: service }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: gw, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
scenarios:
  lag: { chaos: [ { node: db, latency_ms: 30 } ] }
";

    fn build(drawn: Option<u64>) -> (Generation, ScenarioSet) {
        let cfg = Config::from_yaml(YAML).unwrap();
        Generation::build(&cfg, Path::new("."), drawn, Trust::Owner).unwrap()
    }

    #[test]
    fn build_returns_the_generation_and_the_scenarios_it_loaded() {
        let (g, scenarios) = build(None);
        assert_eq!(scenarios.names(), ["lag"]);
        assert_eq!(g.topo.nodes.len(), 3);
        assert!(g.route_keys.is_empty(), "no spec, so no routes");
        assert_eq!(g.counters.len(), g.endpoints.iter().count());
        // two builds of one config agree
        let (again, _) = build(None);
        assert_eq!(g.route_keys, again.route_keys);
        assert_eq!(g.chain_seed, again.chain_seed);
    }

    fn state() -> Arc<crate::state::AppState> {
        let cfg = Config::from_yaml(YAML).unwrap();
        Arc::new(crate::state::AppState::build(&cfg, Path::new("."), "t".into(), 0).unwrap())
    }

    #[test]
    fn a_held_generation_survives_a_swap_and_new_readers_see_the_new_one() {
        let st = state();
        let held = st.generation().unwrap();
        let (next, _) = build(Some(7));
        st.set_generation(Some(Arc::new(next)));
        assert_eq!(held.chain_seed, 42, "the request that took it keeps it");
        assert_eq!(
            st.generation().unwrap().chain_seed,
            7,
            "the next one sees the swap"
        );
    }

    #[tokio::test]
    async fn gen_answers_409_with_no_system_and_the_generation_with_one() {
        use axum::extract::FromRequestParts;
        let st = state();
        let (mut parts, ()) = axum::http::Request::new(()).into_parts();
        let ok = Gen::from_request_parts(&mut parts, &st).await;
        assert!(ok.is_ok_and(|g| g.0.topo.nodes.len() == 3));
        st.set_generation(None);
        let err = Gen::from_request_parts(&mut parts, &st)
            .await
            .err()
            .unwrap();
        assert_eq!(err.0, axum::http::StatusCode::CONFLICT);
        assert!(err.1.0["error"].as_str().unwrap().contains("no system"));
    }

    #[test]
    fn the_chain_seed_is_the_config_seed_or_the_drawn_one() {
        assert_eq!(build(None).0.chain_seed, 42);
        let (g, _) = build(Some(7));
        assert_eq!(
            (g.seed, g.chain_seed),
            (42, 7),
            "seed stays, the chain draw follows"
        );
    }
}
