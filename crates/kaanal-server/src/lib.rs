mod mock;
mod state;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use kaanal_core::Config;
use serde_json::json;

pub use state::Event;

pub struct ServeOpts {
    pub host: IpAddr,
    /// 0 = let the OS choose (tests). Otherwise this port or the next free one up to +20.
    pub port: u16,
    pub token: String,
}

pub struct BoundServer {
    pub addr: SocketAddr,
    /// Same warnings `check` returns, so `up` can print them.
    pub warnings: Vec<String>,
    handle: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl BoundServer {
    pub async fn wait(self) -> std::io::Result<()> {
        self.handle.await.map_err(std::io::Error::other)?
    }
}

/// Validate everything `serve` would load (config, topology, specs, routes) without binding a port.
/// `Ok` carries non-fatal warnings (ignored schema keywords).
pub fn check(cfg: &Config, base_dir: &Path) -> Result<Vec<String>, String> {
    state::AppState::build(cfg, base_dir, String::new(), 0).map(|st| st.warnings)
}

/// 32 random bytes as hex. New on every `kaanal up`.
pub fn new_token() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub async fn serve(cfg: &Config, base_dir: &Path, opts: ServeOpts) -> Result<BoundServer, String> {
    let listener = bind(opts.host, opts.port).await?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let st = Arc::new(state::AppState::build(
        cfg,
        base_dir,
        opts.token,
        addr.port(),
    )?);
    let warnings = st.warnings.clone();
    let app = router(st);
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(BoundServer {
        addr,
        warnings,
        handle,
    })
}

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

fn router(st: Arc<state::AppState>) -> Router {
    Router::new()
        .route(
            "/__kaanal/health",
            get(|| async { Json(json!({ "ok": true })) }),
        )
        .fallback(mock::handle)
        .with_state(st)
}
