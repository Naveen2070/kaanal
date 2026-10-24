//! Loading and checking `kaanal.yaml`: capped reads, positioned config errors, the auth user list.

use crate::{Config, ConfigError, Fail, Located, NodeConfig, Path, PathBuf, err};

/// **Private.** `file:line:col: msg` when the error has a position, else `file: msg` (R6.1).
pub(crate) fn config_err(path: &Path, e: &ConfigError) -> Fail {
    // With a position, Display is `line:col: msg`, giving `file:line:col: msg`.
    let sep = if e.line.is_some() { ":" } else { ": " };
    err(format!("{}{sep}{e}", path.display()))
}

/// **Private.** A load error as `file:line:col: msg`: placed in the endpoints file when it
/// came from there (spec §8), else in `kaanal.yaml` (`src`).
pub(crate) fn located_err(config: &Path, src: &str, e: &Located) -> Fail {
    match &e.file {
        Some(f) => {
            // Only re-read when there is a key path to find, and never more than the 1 MiB
            // the loaders accept (an oversized file's error has no path).
            let text = if e.pos.is_none() && !e.path.is_empty() {
                read_capped(f)
            } else {
                String::new()
            };
            config_err(f, &e.in_source(&text))
        }
        None => config_err(config, &e.in_source(src)),
    }
}

/// **Private.** At most 1 MiB of `f` as text; empty when unreadable, larger or not UTF-8 (the
/// error then prints without a position).
pub(crate) fn read_capped(f: &Path) -> String {
    use std::io::Read as _;
    let mut buf = Vec::new();
    let ok = std::fs::File::open(f)
        .and_then(|file| file.take((1 << 20) + 1).read_to_end(&mut buf))
        .is_ok_and(|n| n <= 1 << 20);
    if ok {
        String::from_utf8(buf).unwrap_or_default()
    } else {
        String::new()
    }
}

/// **Private.** Reads and parses the config; errors print as `file:line:col: msg`
/// (R6.1). Also returns the config's directory, which spec files resolve against, and
/// the source, so later errors can be placed in it.
pub(crate) fn load(path: &Path) -> Result<(Config, PathBuf, String), Fail> {
    let src = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let cfg = Config::from_yaml(&src).map_err(|e| config_err(path, &e))?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    Ok((cfg, dir, src))
}

/// **Private.** How many users the config's auth node has, or `None` without an auth node.
pub(crate) fn auth_users(cfg: &Config) -> Option<usize> {
    cfg.nodes.values().find_map(|n| match n {
        NodeConfig::Auth(a) => Some(a.users.len()),
        _ => None,
    })
}
