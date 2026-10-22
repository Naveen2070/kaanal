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
mod chaos_lease;
mod control;
mod log;
mod log_api;
mod mock;
mod pin_api;
mod pins;
mod record;
mod scenarios;
mod scope;
mod shutdown;
mod state;
mod wire;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, post, put};
use axum::{Router, middleware};
use kaanal_core::chain::RouteSpec;
use kaanal_core::diagnostics::Located;
use kaanal_core::{Config, Topology};

pub use auth::AuthKey;
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
    state::AppState::build(cfg, base_dir, String::new(), 0).map(|st| st.warnings)
}

/// The spec routes of `cfg` (key, owner, inferred op) and its topology, for
/// `kaanal endpoints`. Spec paths resolve against `base_dir`, as in [`check`].
///
/// # Errors
/// A topology error, an unreadable or invalid spec, or a route defined by two services,
/// placed in `kaanal.yaml`.
pub fn spec_routes(cfg: &Config, base_dir: &Path) -> Result<(Topology, Vec<RouteSpec>), Located> {
    state::spec_routes(cfg, base_dir)
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
    let listener = bind(opts.host, opts.port)
        .await
        .map_err(|e| Located::new(Vec::new(), e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| Located::new(Vec::new(), e.to_string()))?;
    let mut st = state::AppState::build(cfg, base_dir, opts.token, addr.port())?;
    // X5: fail closed. `check` never builds an issuer, so `validate` never needs a key.
    st.auth = auth::Issuer::from_config(cfg, opts.auth_key, addr.port())?;
    st.engine = Some(actor::spawn(kaanal_core::replay::Engine::new(&st.topo)));
    let st = Arc::new(st);
    let warnings = st.warnings.clone();
    let drawn_seed = st.drawn_seed;
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
    Ok(BoundServer {
        addr,
        warnings,
        drawn_seed,
        handle,
        shutdown,
    })
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
        .route("/__kaanal/logs", get(log_api::logs))
        .route("/__kaanal/trace/{id}", get(log_api::trace))
        .route("/__kaanal/reset", post(control::reset))
        .route("/__kaanal/shutdown", post(control::shutdown))
        .route("/__kaanal/events", get(control::events))
        .route("/__kaanal/ui", get(control::ui))
        // route_layer: wraps only the control routes above, never the mock fallback.
        // Every control route is guarded here, so a new handler cannot forget authentication.
        .route_layer(middleware::from_fn_with_state(st.clone(), control::guard))
        .fallback(mock::handle)
        .with_state(st)
}
