//! Whose text a config is, and what the files it names may be: [`Trust`], the path rule for a draft
//! that came through the control API ([`resolve`], [`confine`]), how a path is shown in a message
//! ([`shown`]) and the capped read of a spec or endpoints file ([`read_text`]). Moved out of
//! `generation.rs` (R3.8) with its tests; the code is unchanged.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use kaanal_core::diagnostics::{Located, PathSeg, keys};

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
pub(crate) fn resolve(
    base_dir: &Path,
    rel: &str,
    trust: Trust,
    at: &[PathSeg],
) -> Result<PathBuf, Located> {
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
            ) || matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.'))
        });
    let escapes = file
        .canonicalize()
        .ok()
        .zip(base_dir.canonicalize().ok())
        .is_some_and(|(f, b)| !f.starts_with(b));
    if outside || escapes {
        return Err(Located::at_value(
            at.to_vec(),
            format!(
                "'{rel}' must be a path inside the project folder (no absolute path, no '..', no hidden file or folder)"
            ),
        ));
    }
    Ok(file)
}

/// Checks the files a config *text* names (`nodes.*.spec`, `data`, `endpoints`) against the
/// [`Trust::Api`] rule, without building anything: what Save runs, so a draft that validate and
/// Apply would refuse is never written where the owner's `kaanal up` (which takes paths as
/// written) would later trust it.
///
/// The text is read as YAML by the library the loader uses (`serde_yaml_ng`), with anchors and
/// aliases resolved and merge keys left alone (the loader does not merge them either), so every
/// spelling of a path reaches the check as the value the loader will see. It does **not** go through `Config::from_yaml`: a draft that is YAML but
/// fails a semantic check there must still have its paths checked (a check that skipped such
/// text would fail open). Only text that is not YAML at all passes: nobody can load it.
///
/// # Errors
/// The first path that is absolute, has `..`, or leaves the project through a link.
pub(crate) fn confine(text: &str, base_dir: &Path) -> Result<(), Located> {
    use serde_yaml_ng::Value;
    // same preprocessing as `Config::from_yaml`: a BOM-prefixed draft loads, so it must be checked
    let Ok(doc) = serde_yaml_ng::from_str::<Value>(kaanal_core::strip_bom(text)) else {
        return Ok(());
    };
    // no `apply_merge`: serde does not merge `<<` when the loader reads `Config`, so `<<` is an
    // ordinary key there (a node named `<<`); merging here would hide that node's `spec`
    // fail closed: every node is checked whatever type its key is (the loader reads `1:` or
    // `true:` as a name), and a path that is not a scalar is refused, not skipped
    if let Some(nodes) = doc.get("nodes").and_then(Value::as_mapping) {
        for (name, node) in nodes {
            let label = scalar_text(name).unwrap_or_else(|| "?".into());
            if let Some(spec) = node.get("spec") {
                check_path(base_dir, spec, &keys(&["nodes", &label, "spec"]))?;
            }
        }
    }
    for key in ["data", "endpoints"] {
        match doc.get(key) {
            // an inline `endpoints:` map is not a path
            Some(Value::Mapping(_)) if key == "endpoints" => {}
            Some(v) => check_path(base_dir, v, &keys(&[key]))?,
            None => {}
        }
    }
    Ok(())
}

/// **Private.** A scalar's text as the loader would read it for a `String`; `None` for a list or map.
fn scalar_text(v: &serde_yaml_ng::Value) -> Option<String> {
    use serde_yaml_ng::Value;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Tagged(t) => scalar_text(&t.value),
        Value::Null | Value::Sequence(_) | Value::Mapping(_) => None,
    }
}

/// **Private.** Holds the value of a `spec`, `data` or `endpoints` key to [`Trust::Api`]: null is
/// no path; a scalar is resolved as the text the loader would use; anything else is refused.
fn check_path(base_dir: &Path, v: &serde_yaml_ng::Value, at: &[PathSeg]) -> Result<(), Located> {
    if v.is_null() {
        return Ok(());
    }
    let Some(p) = scalar_text(v) else {
        return Err(Located::at_value(
            at.to_vec(),
            "a file path must be text inside the project folder".to_string(),
        ));
    };
    resolve(base_dir, &p, Trust::Api, at).map(drop)
}

/// **Private.** `file` as a message names it: as is for the owner, relative to the project for the API.
pub(crate) fn shown(base_dir: &Path, file: &Path, trust: Trust) -> String {
    if trust == Trust::Owner {
        return file.display().to_string();
    }
    crate::draft::apply::relative(base_dir, file)
}

/// **Private.** The text of a regular file, at most [`MAX_FILE_BYTES`].
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
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

#[cfg(test)]
mod trust_tests {
    use super::*;
    use crate::generation::Generation;
    use kaanal_core::Config;
    use std::path::PathBuf;

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
    fn confine_checks_a_draft_that_starts_with_a_bom() {
        let (dir, _) = project("bom");
        let draft = "\u{feff}version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: '../outside.yaml' }\n";
        assert!(
            Config::from_yaml(draft).is_ok(),
            "the loader accepts the BOM"
        );
        let err = confine(draft, &dir).expect_err("refused");
        assert!(err.msg.contains("inside the project folder"), "{}", err.msg);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn confine_does_not_apply_merge_keys_the_loader_ignores() {
        let (dir, _) = project("merge");
        // the loader reads `<<` as a node *named* `<<`; a merge would hide its spec
        let draft = "version: 1\nnodes:\n  <<: { type: service, spec: '../outside.yaml' }\n  gw: { type: gateway }\nedges:\n  - { from: gw, to: '<<' }\n";
        let err = confine(draft, &dir).expect_err("refused");
        assert!(err.msg.contains("inside the project folder"), "{}", err.msg);
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
