//! The self-validation of generated responses over a config's specs.

use std::path::Path;

use kaanal_core::diagnostics::{Located, keys};
use kaanal_core::{Config, Topology};
use kaanal_data::selfcheck::{SAMPLES, check_model};

use crate::generation::{load_data, read_api};

/// **Private.** Each node's spec is checked with that node's own options (its `locale`, else the
/// project's, plus the project dictionary), as the mock would generate it.
pub(crate) fn run(cfg: &Config, base_dir: &Path) -> Result<Vec<String>, Located> {
    let topo = Topology::build(cfg).map_err(|e| e.located(cfg))?;
    let (options, _) = load_data(cfg, base_dir, &topo, &mut Vec::new())?;
    let mut lines = Vec::new();
    for (id, node) in topo.nodes.iter().enumerate() {
        let Some(spec) = &node.spec else { continue };
        let at = keys(&["nodes", node.name.as_str(), "spec"]);
        let (file, api) = read_api(spec, base_dir, &at)?;
        lines.extend(
            check_model(&api, &|_| options[id].clone(), SAMPLES)
                .into_iter()
                .map(|w| format!("{}: {w}", file.display())),
        );
    }
    Ok(lines)
}
