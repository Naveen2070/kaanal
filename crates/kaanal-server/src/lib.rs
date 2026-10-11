//! The HTTP server: the only crate that knows HTTP.
//!
//! One axum router on one port serves both the control API (`/__kaanal/*`, guarded
//! by token + Host checks) and the mock engine (every other path falls through to
//! the mock handler). The engine itself — chain execution, data generation — is
//! called synchronously inside the request handler; it is pure math and takes
//! microseconds.

#![warn(missing_docs)]

mod auth;
mod control;
mod mock;
mod state;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, put};
use axum::{Router, middleware};
use kaanal_core::chain::RouteSpec;
use kaanal_core::diagnostics::Located;
use kaanal_core::{Config, Topology};

pub use auth::AuthKey;

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
}

impl BoundServer {
    /// Runs until the server stops (Ctrl+C is handled by the caller).
    ///
    /// # Errors
    /// Returns the IO error that stopped the server (e.g. the listener died).
    pub async fn wait(self) -> std::io::Result<()> {
        self.handle.await.map_err(std::io::Error::other)?
    }
}

/// Validate everything `serve` would load (config, topology, specs, endpoints inline or from a file) without binding a port.
/// `Ok` carries non-fatal warnings (ignored schema keywords, nodes with `concurrency`).
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
    let st = Arc::new(st);
    let warnings = st.warnings.clone();
    let drawn_seed = st.drawn_seed;
    let app = router(st);
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(BoundServer {
        addr,
        warnings,
        drawn_seed,
        handle,
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
            get(control::get_chaos).delete(control::clear_all),
        )
        .route(
            "/__kaanal/chaos/{node}",
            put(control::set_chaos).delete(control::clear_node),
        )
        .route("/__kaanal/events", get(control::events))
        .route("/__kaanal/ui", get(control::ui))
        // route_layer: wraps only the control routes above, never the mock fallback.
        // Every control route is guarded here, so a new handler cannot forget authentication.
        .route_layer(middleware::from_fn_with_state(st.clone(), control::guard))
        .fallback(mock::handle)
        .with_state(st)
}
