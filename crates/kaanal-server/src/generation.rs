//! [`Generation`]: everything built from one config (topology, routes, endpoints, data options,
//! engine), the part of the server a later Apply can swap. [`Generation::build`] is the full load
//! pipeline shared by `serve` and `check`: topology → `OpenAPI` specs → endpoints (inline or the
//! endpoints file) → route table. Every problem is a fatal error with a precise message, or, for
//! ignorable schema keywords, a collected warning. Moved out of `state.rs` (R3.8); the only change
//! to the pipeline is that the realistic-mode `drawn_seed` is now a parameter.

use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
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

use crate::scenarios;
use crate::state::Route;

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

/// Whose text the config is. A person who wrote the file on disk (startup, `check`) may point
/// `spec:`, `data:` and `endpoints:` anywhere they can read. Text that came through the control
/// API (a draft) is held to the project folder: the API must not become a way to make the server
/// read other files and report what it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// The file's owner: paths are taken as written; a scenario file that does not resolve is an error.
    Owner,
    /// A draft from the API: paths must stay inside the project folder, messages show paths
    /// relative to it, and a scenario file that does not resolve is flagged instead of failing.
    Api,
}

/// The largest spec or endpoints file read: a bound, so a path to a device or an enormous file
/// cannot exhaust memory.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// **Private.** `rel` (a path written in the config) under `base_dir`. For [`Trust::Api`] it must
/// be relative, without `..`, and what it names must not leave `base_dir` through a link. The
/// message repeats only what the draft itself said.
fn resolve(base_dir: &Path, rel: &str, trust: Trust, at: &[PathSeg]) -> Result<PathBuf, Located> {
    let file = base_dir.join(rel);
    if trust == Trust::Owner {
        return Ok(file);
    }
    let p = Path::new(rel);
    let outside = p.is_absolute()
        || p.has_root()
        || p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        });
    let escapes = file
        .canonicalize()
        .ok()
        .zip(base_dir.canonicalize().ok())
        .is_some_and(|(f, b)| !f.starts_with(b));
    if outside || escapes {
        return Err(Located::at_value(
            at.to_vec(),
            format!("'{rel}' must be a path inside the project folder (no absolute path, no '..')"),
        ));
    }
    Ok(file)
}

/// Checks the files a config *text* names (`nodes.*.spec`, `data`, `endpoints`) against the
/// [`Trust::Api`] rule, without building anything: what Save runs, so a draft that validate and
/// Apply would refuse is never written where the owner's `kaanal up` (which takes paths as
/// written) would later trust it. Text that is not YAML cannot be loaded by anyone, so it passes.
///
/// # Errors
/// The first path that is absolute, has `..`, or leaves the project through a link.
pub(crate) fn confine(text: &str, base_dir: &Path) -> Result<(), Located> {
    let Ok(leaves) = kaanal_core::splice::scalar_paths(text) else {
        return Ok(());
    };
    for (path, raw) in leaves {
        let named = matches!(
            path.as_slice(),
            [PathSeg::Key(a), PathSeg::Key(_), PathSeg::Key(c)] if a == "nodes" && c == "spec"
        ) || matches!(path.as_slice(), [PathSeg::Key(k)] if k == "data" || k == "endpoints");
        if named {
            // the leaf is source text (quotes included): read it as the scalar it is
            let rel = serde_yaml_ng::from_str::<String>(&raw).unwrap_or(raw);
            resolve(base_dir, &rel, Trust::Api, &path)?;
        }
    }
    Ok(())
}

/// **Private.** `file` as a message names it: as is for the owner, relative to the project for the API.
fn shown(base_dir: &Path, file: &Path, trust: Trust) -> String {
    if trust == Trust::Owner {
        return file.display().to_string();
    }
    crate::draft::apply::relative(base_dir, file)
}

/// **Private.** The text of a regular file, at most [`MAX_FILE_BYTES`].
fn read_text(path: &Path) -> std::io::Result<String> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::other(format!(
            "larger than {} MiB",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES)
        .read_to_string(&mut text)?;
    Ok(text)
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
    let api = ApiModel::from_openapi(&src)
        .map_err(|e| Located::new(at.to_vec(), format!("{label}: {e}")))?;
    Ok((file, api))
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

#[cfg(test)]
mod trust_tests {
    use super::*;

    const SPEC: &str = "openapi: 3.0.0\ninfo: { title: t, version: '1' }\npaths:\n  /ping:\n    get:\n      responses:\n        '200': { description: ok }\n";

    /// A project folder with a spec one level above it.
    fn project(name: &str) -> (PathBuf, Config) {
        let root = std::env::temp_dir().join(format!("kaanal-trust-{name}-{}", std::process::id()));
        let dir = root.join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("outside.yaml"), SPEC).unwrap();
        let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: '../outside.yaml' }\nedges:\n  - { from: gw, to: app }\n";
        (dir, Config::from_yaml(yaml).unwrap())
    }

    #[test]
    fn the_owner_may_point_outside_the_project_and_the_api_may_not() {
        let (dir, cfg) = project("owner-api");
        assert!(Generation::build(&cfg, &dir, None, Trust::Owner).is_ok());
        let err = Generation::build(&cfg, &dir, None, Trust::Api)
            .err()
            .expect("refused");
        assert!(err.msg.contains("inside the project folder"), "{}", err.msg);
        assert!(!err.msg.contains(&dir.display().to_string()), "{}", err.msg);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_spec_larger_than_the_limit_is_not_read() {
        let (dir, _) = project("big");
        let big = dir.join("big.yaml");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        let err = read_text(&big).unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
        assert!(
            read_text(&dir)
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
