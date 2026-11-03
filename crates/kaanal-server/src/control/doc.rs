//! The draft as JSON for Studio: `doc`, `project_id`, `kind_defaults`, and `GET /__kaanal/demo`.

use super::*;
use crate::draft::apply::Diagnostic;
use kaanal_core::{Config, NodeKind, presets, strip_bom};
use sha2::{Digest, Sha256};

/// **Private.** The draft text as JSON (the file's own keys, no new schema), or the diagnostic that
/// says why it is not a config. `Config::from_yaml` decides what a valid shape is.
pub(super) fn draft_doc(text: &str) -> (Option<Value>, Option<Diagnostic>) {
    if let Err(e) = Config::from_yaml(text) {
        return (None, Some(Diagnostic::parsed(e)));
    }
    // from_yaml accepted it (and bounded its aliases), so this only fails on something JSON cannot hold (a non-string key)
    match serde_yaml_ng::from_str::<Value>(strip_bom(text)) {
        Ok(v) => (Some(v), None),
        Err(e) => (
            None,
            Some(Diagnostic {
                file: None,
                line: e.location().map(|l| l.line()),
                col: e.location().map(|l| l.column().saturating_sub(1)),
                path: Vec::new(),
                message: e.to_string(),
            }),
        ),
    }
}

/// **Private.** A random id drawn once per server process: `rev` restarts at 0 with the process,
/// so Studio pairs it with `rev` to tell a restarted server from an unchanged draft.
pub(super) fn boot_id() -> &'static str {
    static BOOT: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| format!("{:016x}", rand::random::<u64>()));
    &BOOT
}

/// **Private.** First 16 hex characters of the SHA-256 of the config file's absolute path, so
/// Studio can tell projects apart without learning where one lives; `"none"` with no file.
pub(super) fn project_id(file: Option<&std::path::Path>) -> String {
    let Some(f) = file else {
        return "none".into();
    };
    let h = Sha256::digest(f.to_string_lossy().as_bytes());
    h.iter().take(8).fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// **Private.** The name of the folder the config file sits in (a base name, never a path), cut
/// to 80 characters, for the top bar's heading; `None` with no file or a folder with no name.
pub(super) fn project_name(file: Option<&std::path::Path>) -> Option<String> {
    let dir = file?.parent()?.file_name()?.to_string_lossy();
    let name: String = dir.chars().take(80).collect();
    (!name.trim().is_empty()).then_some(name)
}

/// **Private.** Per node kind, what a Studio-created node starts as: parsed from
/// [`presets::starter`], the one place those capacities are defined.
pub(super) fn kind_defaults() -> &'static Value {
    static CELL: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CELL.get_or_init(build_kind_defaults)
}

fn build_kind_defaults() -> Value {
    let mut m = Map::new();
    for k in NodeKind::ALL {
        let v = serde_yaml_ng::from_str(presets::starter(k)).unwrap_or(Value::Null);
        m.insert(k.as_str().into(), v);
    }
    Value::Object(m)
}

/// **Private.** The rules of each node kind that Studio needs and must not keep its own copy of:
/// `[{ kind, callees: [kind], service_time }]` in the order of [`NodeKind::ALL`]. A new kind is
/// listed here without a change to this function.
pub(super) fn kinds() -> &'static Value {
    static CELL: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        Value::Array(
            NodeKind::ALL
                .into_iter()
                .map(|k| {
                    let callees: Vec<&str> = k.callees().iter().map(|c| c.as_str()).collect();
                    json!({ "kind": k.as_str(), "callees": callees, "service_time": k.has_proc() })
                })
                .collect(),
        )
    })
}

/// `GET /__kaanal/demo` — `{ text }`, Studio's self-contained starter system.
pub async fn get_demo() -> Json<Value> {
    Json(json!({ "text": crate::STUDIO_DEMO_YAML }))
}
