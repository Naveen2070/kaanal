//! The HTTP server: the only crate that knows HTTP.
//!
//! One axum router on one port serves both the control API (`/__kaanal/*`, guarded
//! by token + Host checks) and the mock engine (every other path falls through to
//! the mock handler). A handler samples its request's plan (pure math, microseconds) and
//! hands it to the replay engine, which runs on one actor task (the `actor` module) so queues, pools
//! and rate limits are shared by every request.

#![warn(missing_docs)]

mod actor;
mod auth;
mod body;
mod chaos_lease;
mod collections;
mod control;
mod draft;
mod generation;
mod live;
mod log;
mod log_api;
mod mock;
mod paths;
mod payload;
mod pin_api;
mod pins;
mod record;
mod scenarios;
mod scope;
mod selfcheck;
mod shutdown;
mod sim;
mod site;
mod state;
mod wire;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get, post, put};
use axum::{Router, middleware};
use kaanal_core::chain::RouteSpec;
use kaanal_core::diagnostics::Located;
use kaanal_core::{Config, Topology};

pub use auth::AuthKey;
pub use collections::{Collections, MAX_ITEM_BYTES, MAX_ITEMS};
pub use control::{Asset, Bundle};
pub use draft::{BLANK_TEXT, DraftSource};

/// Studio's starter text, one self-contained file (`GET /__kaanal/demo` serves it). The
/// `kaanal demo` project, with its spec, scenarios and site, lives in `crates/kaanal/demo`.
pub const STUDIO_DEMO_YAML: &str = include_str!("../demo/studio.yaml");
/// The data-file schemas, for `kaanal schema dictionary|pack`.
pub use kaanal_data::{dictionary_schema, pack_schema};
pub use log::{
    ChainErrorRec, Filter, HopRec, IdGen, LOG_CAP, LogEntry, LogRing, Lookup, MAX_HOPS, MAX_TRACE,
    Page, StatusMatch, client_id, format_id, matches, parse_id,
};

/// Options for [`serve`].
pub struct ServeOpts {
    /// Address to bind. `127.0.0.1` by default; anything else prints a warning upstream.
    pub host: IpAddr,
    /// 0 = let the OS choose (tests). Otherwise this port or the next free one up to +20.
    pub port: u16,
    /// Bearer token the control API requires (see [`new_token`]).
    pub token: String,
    /// The key that signs an auth node's tokens (`kaanal up` loads or creates it). Required
    /// when the config has an auth node: [`serve`] fails closed without it. Ignored otherwise.
    pub auth_key: Option<AuthKey>,
    /// The config file and its text, to start the Studio draft from (`kaanal up` passes them).
    /// `None`: the draft starts empty.
    pub draft: Option<DraftSource>,
}

/// A running server returned by [`serve`].
pub struct BoundServer {
    /// The address actually bound (after the +20 port search).
    pub addr: SocketAddr,
    /// Same warnings `check` returns, so `up` can print them.
    pub warnings: Vec<String>,
    /// The chain seed drawn at startup under `determinism: realistic`; print it so the run can
    /// be replayed as `controlled`. `None` under `controlled`.
    pub drawn_seed: Option<u64>,
    /// **Private.** The spawned `axum::serve` task.
    handle: tokio::task::JoinHandle<std::io::Result<()>>,
    /// **Private.** Shared with the server state, so the route and [`BoundServer::shutdown`] are one.
    shutdown: shutdown::Shutdown,
}

impl BoundServer {
    /// Runs until the server stops (Ctrl+C is handled by the caller), or until the deadline
    /// of a shutdown request passes; then the serve task is aborted and this returns `Ok`.
    ///
    /// # Errors
    /// Returns the IO error that stopped the server (e.g. the listener died).
    pub async fn wait(mut self) -> std::io::Result<()> {
        tokio::select! {
            done = &mut self.handle => done.map_err(std::io::Error::other)?,
            () = self.shutdown.expired() => {
                self.handle.abort();
                Ok(())
            }
        }
    }

    /// Starts a graceful stop, as `POST /__kaanal/shutdown` does: in-flight requests get `budget`
    /// to finish, then what is left is cut. The first request fixes the deadline; later ones never
    /// extend it. Returns the time left to it.
    pub fn shutdown(&self, budget: std::time::Duration) -> std::time::Duration {
        self.shutdown.request(budget)
    }

    /// [`BoundServer::shutdown`] as an owned closure, for a caller that has handed `self` to
    /// [`BoundServer::wait`] (which consumes it) and still wants to start the stop, e.g. on a signal.
    pub fn stopper(
        &self,
    ) -> impl Fn(std::time::Duration) -> std::time::Duration + Send + Sync + use<> {
        let sd = self.shutdown.clone();
        move |budget| sd.request(budget)
    }
}

/// Validate everything `serve` would load (config, topology, specs, endpoints inline or from a file) without binding a port.
/// `Ok` carries non-fatal warnings (ignored schema keywords).
///
/// # Errors
/// Returns the first load error (bad config, unreadable or invalid spec, route conflict),
/// with its place in `kaanal.yaml` (see [`Located::in_source`]).
pub fn check(cfg: &Config, base_dir: &Path) -> Result<Vec<String>, Located> {
    generation::Generation::build(cfg, base_dir, None, generation::Trust::Owner)
        .map(|(g, _)| g.warnings)
}

/// Samples every spec endpoint's generated response and validates it against its schema
/// (FR-DATA-05). Findings are warnings, one line per endpoint, naming the spec file. Slower than
/// [`check`] on a big spec, so `kaanal up` runs it after the port is bound.
///
/// # Errors
/// The same load errors as [`check`].
pub fn selfcheck(cfg: &Config, base_dir: &Path) -> Result<Vec<String>, Located> {
    selfcheck::run(cfg, base_dir)
}

/// The spec routes of `cfg` (key, owner, inferred op) and its topology, for
/// `kaanal endpoints`. Spec paths resolve against `base_dir`, as in [`check`].
///
/// # Errors
/// A topology error, an unreadable or invalid spec, or a route defined by two services,
/// placed in `kaanal.yaml`.
pub fn spec_routes(cfg: &Config, base_dir: &Path) -> Result<(Topology, Vec<RouteSpec>), Located> {
    generation::spec_routes(cfg, base_dir)
}

/// 32 random bytes as hex. New on every `kaanal up`.
pub fn new_token() -> String {
    use std::fmt::Write as _;
    rand::random::<[u8; 32]>()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            write!(s, "{b:02x}").expect("writing to a String cannot fail");
            s
        })
}

/// Binds the port, builds server state, spawns the axum server on
/// the current tokio runtime, and returns its handle.
///
/// # Errors
/// Returns a [`Located`] error if no port in `port..=port+20` is free or if
/// building server state fails (bad config, specs or routes), or if the config has an auth
/// node and `opts.auth_key` is `None`.
pub async fn serve(cfg: &Config, base_dir: &Path, opts: ServeOpts) -> Result<BoundServer, Located> {
    serve_with_studio(cfg, base_dir, opts, Bundle::embedded()).await
}

/// [`serve`] with `studio` as the Studio bundle instead of the one compiled in
/// ([`Bundle::embedded`]); `None` serves the status page at `/__kaanal/studio` and no assets.
/// Tests use it to serve a fixture bundle.
///
/// # Errors
/// As [`serve`].
pub async fn serve_with_studio(
    cfg: &Config,
    base_dir: &Path,
    opts: ServeOpts,
    studio: Option<Bundle>,
) -> Result<BoundServer, Located> {
    let (listener, addr) = open(&opts).await?;
    let mut st = state::AppState::build(cfg, base_dir, opts.token, addr.port())?;
    st.studio = studio;
    // X5: fail closed. `check` never builds an issuer, so `validate` never needs a key.
    st.draft = std::sync::Mutex::new(draft::Draft::from_source(opts.draft));
    st.auth_key.clone_from(&opts.auth_key);
    // X5: fail closed. `check` never builds an issuer, so `validate` never needs a key.
    let auth = auth::Issuer::from_config(cfg, opts.auth_key, addr.port())?;
    let g = st.gen_mut();
    g.auth = auth;
    g.engine = Some(actor::spawn(kaanal_core::replay::Engine::live(&g.topo)));
    let warnings = g.warnings.clone();
    let drawn_seed = st.drawn_seed;
    Ok(launch(st, listener, addr, warnings, drawn_seed))
}

/// Starts with **no system**: the control API and `/__kaanal/ui` work, every other path answers
/// 404 `{"error":"no system applied yet"}`, and the Studio draft is [`draft::BLANK_TEXT`], to be
/// saved at `file`. The first `POST /__kaanal/draft/apply` that validates makes generation 1.
/// `opts.draft` is ignored (there is nothing to start from).
///
/// # Errors
/// Returns a [`Located`] error if no port in `port..=port+20` is free.
pub async fn serve_blank(
    base_dir: &Path,
    file: &Path,
    opts: ServeOpts,
) -> Result<BoundServer, Located> {
    let (listener, addr) = open(&opts).await?;
    let mut st = state::AppState::blank(base_dir, opts.token, addr.port());
    st.draft = std::sync::Mutex::new(draft::Draft::blank(file));
    st.auth_key = opts.auth_key;
    // no seed line to print: there is no config whose determinism it would belong to
    Ok(launch(st, listener, addr, Vec::new(), None))
}

/// **Private.** Binds the port of `opts` and returns the listener and its address.
async fn open(opts: &ServeOpts) -> Result<(tokio::net::TcpListener, SocketAddr), Located> {
    let listener = bind(opts.host, opts.port)
        .await
        .map_err(|e| Located::new(Vec::new(), e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| Located::new(Vec::new(), e.to_string()))?;
    Ok((listener, addr))
}

/// **Private.** Shares the state, starts the lease task and the axum server, and returns the handle.
/// `drawn_seed` is what the caller prints (the state keeps its own).
fn launch(
    st: state::AppState,
    listener: tokio::net::TcpListener,
    addr: SocketAddr,
    warnings: Vec<String>,
    drawn_seed: Option<u64>,
) -> BoundServer {
    let st = Arc::new(st);
    let shutdown = st.shutdown.clone();
    // Detached; it stops when a shutdown is requested (no lease should fire during the drain).
    let sd = shutdown.clone();
    let leases = control::lease_task(Arc::clone(&st));
    tokio::spawn(async move {
        tokio::select! {
            () = leases => {}
            () = sd.requested() => {}
        }
    });
    let app = router(st);
    let listener = shutdown::Cut::new(listener, shutdown.clone());
    let sd = shutdown.clone();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { sd.requested().await })
            .await
    });
    BoundServer {
        addr,
        warnings,
        drawn_seed,
        handle,
        shutdown,
    }
}

/// **Private.** Binds the requested port, else the next free one up to +20.
async fn bind(host: IpAddr, port: u16) -> Result<tokio::net::TcpListener, String> {
    let tries: u16 = if port == 0 { 1 } else { 21 };
    let mut last = String::new();
    for p in (0..tries).map(|i| port.saturating_add(i)) {
        match tokio::net::TcpListener::bind((host, p)).await {
            Ok(l) => return Ok(l),
            Err(e) => last = format!("{host}:{p}: {e}"),
        }
    }
    Err(format!(
        "no free port in {port}..={}: {last}",
        port.saturating_add(20)
    ))
}

/// **Private.** Builds the route table: control routes first (wrapped in the
/// `guard` middleware via `route_layer`, so a new control route cannot forget
/// authentication), then the mock fallback.
fn router(st: Arc<state::AppState>) -> Router {
    Router::new()
        .route(
            "/__kaanal/health",
            get(|| async { axum::Json(serde_json::json!({ "ok": true })) }),
        )
        .route("/__kaanal/topology", get(control::topology))
        .route(
            "/__kaanal/chaos",
            get(control::get_chaos)
                .put(control::set_chaos_keyed)
                .delete(control::clear_keyed),
        )
        .route(
            "/__kaanal/chaos/{node}",
            put(control::set_chaos).delete(control::clear_node),
        )
        .route(
            "/__kaanal/scenarios",
            get(control::list_scenarios).delete(control::clear_scenarios),
        )
        // Static `active` and `save` win over the `{name}` capture below (matchit prefers static segments).
        .route(
            "/__kaanal/scenarios/active",
            get(control::get_active).put(control::set_active),
        )
        .route("/__kaanal/scenarios/save", post(control::save_scenario))
        .route(
            "/__kaanal/scenarios/{name}",
            get(control::get_scenario)
                .put(control::put_scenario)
                .delete(control::delete_scenario),
        )
        .route("/__kaanal/scenarios/{name}/activate", post(control::activate))
        .route(
            "/__kaanal/scenarios/{name}/deactivate",
            post(control::deactivate),
        )
        .route(
            "/__kaanal/pins",
            get(pin_api::get_pins)
                .put(pin_api::put_pin)
                .delete(pin_api::delete_pins),
        )
        .route("/__kaanal/cycle", post(pin_api::cycle))
        .route(
            "/__kaanal/payload",
            get(control::list_payloads)
                .put(control::put_payload)
                .delete(control::delete_payload),
        )
        .route("/__kaanal/instances", get(control::get_instances))
        .route("/__kaanal/storage-prices", get(control::get_storage_prices))
        .route("/__kaanal/prices", get(control::get_price_regions))
        .route("/__kaanal/prices/{provider}/{region}", get(control::get_prices))
        .route("/__kaanal/payload/stage-file", post(control::stage_file))
        .route(
            "/__kaanal/payload/preview",
            get(control::payload_preview),
        )
        .route("/__kaanal/logs", get(log_api::logs))
        .route("/__kaanal/trace/{id}", get(log_api::trace))
        .route(
            "/__kaanal/demo",
            get(control::get_demo),
        )
        .route(
            "/__kaanal/draft",
            get(control::get_draft)
                .put(control::put_draft)
                .layer(DefaultBodyLimit::max(draft::MAX_DRAFT_BODY)),
        )
        .route(
            "/__kaanal/draft/validate",
            post(control::validate_draft).layer(DefaultBodyLimit::max(draft::MAX_DRAFT_BODY)),
        )
        .route("/__kaanal/draft/apply", post(control::apply_draft))
        .route("/__kaanal/draft/save", post(control::save_draft))
        .route("/__kaanal/draft/reload", post(control::reload_draft))
        .route("/__kaanal/openapi/skeleton", get(control::get_skeleton))
        .route("/__kaanal/sample", post(control::post_sample))
        .route("/__kaanal/sim/run", post(control::post_run))
        .route("/__kaanal/sim/sweep", post(control::post_sweep))
        .route("/__kaanal/sim/runs", get(control::list_runs))
        .route("/__kaanal/sim/runs/{id}", get(control::get_run))
        .route("/__kaanal/sim/runs/{id}/log", get(control::get_run_log))
        .route("/__kaanal/reset", post(control::reset))
        .route("/__kaanal/shutdown", post(control::shutdown))
        .route("/__kaanal/events", get(control::events))
        .route("/__kaanal/live", get(live::stream::live))
        .route("/__kaanal/live/pause", post(live::stream::pause))
        .route("/__kaanal/live/resume", post(live::stream::resume))
        .route("/__kaanal/live/reset", post(live::stream::reset))
        .route("/__kaanal/live/rate", put(live::stream::rate))
        .route("/__kaanal/ui", get(control::ui))
        .route("/__kaanal/studio", get(control::studio))
        // `any` + a catch-all so every method and every path under `studio/` (`assets%2Fx`,
        // `assets/`, `x/../y`) reaches the guard (401 without a token) instead of a route-less
        // 404/405; the handler serves only an exact table hit on GET/HEAD (R1.106).
        .route("/__kaanal/studio/{*rest}", any(control::studio_asset))
        // the catch-all never matches an empty rest
        .route("/__kaanal/studio/", any(control::studio_asset))
        // route_layer: wraps only the control routes above, never the mock fallback.
        // Every control route is guarded here, so a new handler cannot forget authentication.
        .route_layer(middleware::from_fn_with_state(st.clone(), control::guard))
        .fallback(mock::handle)
        .with_state(st)
}
