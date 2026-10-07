mod state;

use std::path::Path;

use kaanal_core::Config;

pub use state::Event;

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
