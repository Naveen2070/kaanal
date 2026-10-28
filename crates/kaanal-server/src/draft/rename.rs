//! `rename_node`: every place a node's name appears in the draft, changed together.
//!
//! A node name is written in: the `nodes:` key, `edges` (`from`, `to`), the steps of node
//! `chains:` and of inline `endpoints:` `chain:` lists (`call`, `cache`, or a bare name), the keys
//! of an endpoint's `error_map`, and inline `scenarios:` (`chaos[].node` and `chaos[].edge`).
//! The files outside `kaanal.yaml` (an `endpoints:` file, `scenarios/*.yaml`) are not part of the
//! draft: Apply lists the scenario files that still name the old node, and an endpoints file is
//! reported in `notes`.

use super::patch::scalar;
use kaanal_core::PathSeg::{self, Index, Key};
use kaanal_core::splice::{Edit, scalar_paths};
use std::fmt::Write as _;

/// What a rename does: the edits, where they land, and what it cannot do for the caller.
pub struct Plan {
    /// The splice edits, one per place.
    pub edits: Vec<Edit>,
    /// A label for each place (`nodes.db`, `edges[1].to`).
    pub touched: Vec<String>,
    /// Files this rename does not reach.
    pub notes: Vec<String>,
}

/// **Private.** A path as a label.
fn label(path: &[PathSeg]) -> String {
    let mut s = String::new();
    for seg in path {
        match seg {
            Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            Index(i) => {
                let _ = write!(s, "[{i}]");
            }
        }
    }
    s
}

/// **Private.** The key at `i` of `path`, if it is one.
fn key(path: &[PathSeg], i: usize) -> Option<&str> {
    match path.get(i) {
        Some(Key(k)) => Some(k),
        _ => None,
    }
}

/// **Private.** Whether the leaf at `path` is a node reference: an edge end, a chaos entry's node
/// or edge end, or a step of a chain (`call`, `cache`, or a bare name in a step list).
fn is_reference(path: &[PathSeg]) -> bool {
    let last = path.last();
    let end = |k: &PathSeg| matches!(k, Key(x) if x == "from" || x == "to");
    match key(path, 0) {
        Some("edges") => path.len() == 3 && matches!(path[1], Index(_)) && end(&path[2]),
        Some("scenarios") => {
            key(path, 2) == Some("chaos")
                && matches!(path.get(3), Some(Index(_)))
                && match path.len() {
                    5 => matches!(path[4], Key(ref x) if x == "node"),
                    6 => key(path, 4) == Some("edge") && end(&path[5]),
                    _ => false,
                }
        }
        Some("nodes") if key(path, 2) == Some("chains") && path.len() > 4 => step_end(last),
        Some("endpoints") if key(path, 2) == Some("chain") => step_end(last),
        _ => false,
    }
}

/// **Private.** The last step of a leaf in a step list: `call`, `cache` or a list item (a bare name).
fn step_end(last: Option<&PathSeg>) -> bool {
    matches!(last, Some(Index(_))) || matches!(last, Some(Key(k)) if k == "call" || k == "cache")
}

/// **Private.** Whether `path` is inside `prefix`.
fn under(path: &[PathSeg], prefix: &[PathSeg]) -> bool {
    path.starts_with(prefix)
}

/// Plans the rename of node `from` to `to` in `text`.
///
/// # Errors
/// `from` is not a node, `to` is one already (or empty, or the same), or the text is not YAML.
pub fn plan(text: &str, from: &str, to: &str) -> Result<Plan, String> {
    if to.is_empty() || to == from {
        return Err("rename_node needs a new name that differs from the old one".into());
    }
    let leaves = scalar_paths(text).map_err(|e| e.to_string())?;
    let node = |name: &str| vec![Key("nodes".into()), Key(name.into())];
    if !leaves.iter().any(|(p, _)| under(p, &node(from))) {
        return Err(format!("no node named '{from}'"));
    }
    if leaves.iter().any(|(p, _)| under(p, &node(to))) {
        return Err(format!("a node named '{to}' already exists"));
    }
    let mut edits = vec![Edit::RenameKey {
        path: node(from),
        to: to.to_string(),
    }];
    let mut touched = vec![label(&node(from))];
    let mut keys_done: Vec<Vec<PathSeg>> = Vec::new();
    for (path, raw) in &leaves {
        // an endpoint's error_map is keyed by the failing node
        if key(path, 0) == Some("endpoints")
            && key(path, 2) == Some("error_map")
            && key(path, 3) == Some(from)
        {
            let k = path[..4].to_vec();
            if !keys_done.contains(&k) {
                touched.push(label(&k));
                edits.push(Edit::RenameKey {
                    path: k.clone(),
                    to: to.to_string(),
                });
                keys_done.push(k);
            }
        }
        // the leaf is source text (quotes included): read it as the scalar it is
        let value = serde_yaml_ng::from_str::<String>(raw).unwrap_or_else(|_| raw.clone());
        if value == from && is_reference(path) {
            touched.push(label(path));
            edits.push(Edit::Set {
                path: path.clone(),
                value: scalar(to),
            });
        }
    }
    let notes = leaves
        .iter()
        .filter(|(p, _)| p.as_slice() == [Key("endpoints".into())])
        .map(|_| {
            "the endpoints file is not part of the draft: change '".to_string()
                + from
                + "' to '"
                + to
                + "' in it"
        })
        .collect();
    Ok(Plan {
        edits,
        touched,
        notes,
    })
}

/// The text of a scenario file (`chaos:` entries) with each rename in `renames` applied to the
/// nodes it names (`node:`, and the ends of `edge:`), in order, so `a -> b` then `b -> c` ends at
/// `c`. `None` when the file names none of them.
///
/// # Errors
/// The file is not YAML this module reads, or a name in it is in an anchored value (changing
/// it would change its copies): the file is then left as it is and the caller says so.
pub fn rewrite_scenario(
    text: &str,
    renames: &[(String, String)],
) -> Result<Option<String>, String> {
    let leaves = scalar_paths(text).map_err(|e| e.to_string())?;
    let mut edits = Vec::new();
    for (path, raw) in &leaves {
        let end = |k: &PathSeg| matches!(k, Key(x) if x == "from" || x == "to");
        let is_ref = key(path, 0) == Some("chaos")
            && matches!(path.get(1), Some(Index(_)))
            && match path.len() {
                3 => matches!(path[2], Key(ref x) if x == "node"),
                4 => key(path, 2) == Some("edge") && end(&path[3]),
                _ => false,
            };
        if !is_ref {
            continue;
        }
        let value = serde_yaml_ng::from_str::<String>(raw).unwrap_or_else(|_| raw.clone());
        let now = renames.iter().fold(
            value.clone(),
            |v, (from, to)| if v == *from { to.clone() } else { v },
        );
        if now != value {
            edits.push(Edit::Set {
                path: path.clone(),
                value: scalar(&now),
            });
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let done = kaanal_core::splice::splice(text, &edits).map_err(|e| e.to_string())?;
    if done.lost.is_empty() {
        Ok(Some(done.text))
    } else {
        Err(done.lost.join("; "))
    }
}
