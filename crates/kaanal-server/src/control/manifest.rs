//! Reads the Vite manifest (`ui-dist/.vite/manifest.json`) and generates Studio's asset table
//! (R1.106). Shared by `build.rs` (through `#[path]`) and the unit tests, so it uses only `std`
//! and `serde_json`.
//!
//! The table holds exactly the files the manifest references (never a directory walk), with an
//! extension allowlist; `index.html`, dotfiles, `.map` and the manifest itself are never in it,
//! and a referenced file that is missing is an error (the build fails).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The extensions an asset may have; anything else the manifest references is left out.
pub const EXTENSIONS: [&str; 5] = ["js", "css", "woff2", "svg", "png"];

/// One file of the table: its name under `assets/` and whether Vite hashed it.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name under `assets/` (one path segment, `[A-Za-z0-9._-]`, not starting with `.`).
    pub name: String,
    /// True when the manifest shows the name was hashed (see [`entries`]).
    pub immutable: bool,
}

/// The table's files, sorted by name, from the manifest text `manifest` of the bundle in `dist`.
///
/// Only a chunk's own `file` votes on whether it is hashed: its name without extension must be
/// the base (the `src` basename without extension, else the chunk's `name`) plus `-` and a
/// non-empty hash. A chunk with neither has no vote. `css` and `assets` lists never vote (the
/// file is unrelated to the chunk's source; its own entry, if any, votes). A file is immutable
/// only with at least one vote and every vote saying hashed; otherwise `no-cache`, the safe side.
///
/// # Errors
/// The manifest is not a JSON object, an allowlisted file is not directly under `assets/` with a
/// plain name, or a referenced file is missing.
pub fn entries(manifest: &str, dist: &Path) -> Result<Vec<Entry>, String> {
    let json: serde_json::Value =
        serde_json::from_str(manifest).map_err(|e| format!("manifest: {e}"))?;
    let chunks = json.as_object().ok_or("manifest: not a JSON object")?;
    // name -> Some(hashed by every voting reference) / None (no reference could tell)
    let mut table: BTreeMap<String, Option<bool>> = BTreeMap::new();
    // The key (`_` + output name for a srcless chunk) never decides anything.
    for chunk in chunks.values() {
        let base = match (chunk["src"].as_str(), chunk["name"].as_str()) {
            (Some(src), _) => Some(stem(src.rsplit('/').next().unwrap_or(src))),
            (None, name) => name,
        };
        let strs = |field: &str| -> Vec<&str> {
            chunk[field]
                .as_array()
                .map_or_default(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        };
        let mut refs: Vec<(&str, bool)> = chunk["file"]
            .as_str()
            .map(|f| (f, true))
            .into_iter()
            .collect();
        refs.extend(strs("css").into_iter().map(|f| (f, false)));
        refs.extend(strs("assets").into_iter().map(|f| (f, false)));
        for (file, votes) in refs {
            let Some(name) = table_name(file, dist)? else {
                continue;
            };
            let slot = table.entry(name.clone()).or_insert(None);
            if let (true, Some(base)) = (votes, base) {
                *slot = Some(slot.unwrap_or(true) && hashed(stem(&name), base));
            }
        }
    }
    Ok(table
        .into_iter()
        .map(|(name, hashed)| Entry {
            name,
            immutable: hashed.unwrap_or(false),
        })
        .collect())
}

/// **Private.** The table name of a manifest reference, `None` when it is left out (not an
/// allowlisted extension, or a dotfile, `.map`, `index.html` or the manifest, which all fail the
/// allowlist or the dot rule).
///
/// # Errors
/// An allowlisted file not directly under `assets/` with a plain name, or one that is missing.
fn table_name(file: &str, dist: &Path) -> Result<Option<String>, String> {
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    if !EXTENSIONS.contains(&ext) {
        return Ok(None);
    }
    if file.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(format!(
            "{file}: a Studio asset path must not contain `.` or `..` segments"
        ));
    }
    if file.split('/').any(|seg| seg.starts_with('.')) {
        return Ok(None);
    }
    let plain = |n: &str| {
        !n.is_empty()
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    let name = file
        .strip_prefix("assets/")
        .filter(|n| plain(n))
        .ok_or_else(|| {
            format!("{file}: Studio assets must be plain names directly under assets/")
        })?;
    if !dist.join(file).is_file() {
        return Err(format!(
            "{file}: the manifest references it but it is missing"
        ));
    }
    Ok(Some(name.to_string()))
}

/// **Private.** True when the output stem `out` is `base` + `-` + a non-empty hash.
fn hashed(out: &str, base: &str) -> bool {
    out.strip_prefix(base)
        .and_then(|r| r.strip_prefix('-'))
        .is_some_and(|h| !h.is_empty())
}

/// **Private.** A file name without its last extension.
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// The `Content-Type` for an allowlisted extension.
pub fn content_type(ext: &str) -> &'static str {
    match ext {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "svg" => "image/svg+xml",
        _ => "image/png",
    }
}

/// `FNV-1a` 64 of `bytes` as 16 hex digits: the `ETag` base. Not a security hash; it only has to
/// change when the file does.
pub fn fnv1a(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{h:016x}")
}

/// The Rust source of the embedded bundle (`const EMBEDDED: Option<Bundle>`) and every file it
/// embeds (for `rerun-if-changed`), or `None` when `dist` has no manifest (Studio is then the
/// status page).
///
/// # Errors
/// As [`entries`], plus a missing `index.html` or an unreadable file.
pub fn generate(dist: &Path) -> Result<Option<(String, Vec<PathBuf>)>, String> {
    let manifest = dist.join(".vite/manifest.json");
    if !manifest.is_file() {
        return Ok(None);
    }
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let text = String::from_utf8(read(&manifest)?).map_err(|e| format!("manifest: {e}"))?;
    let page = dist.join("index.html");
    if !page.is_file() {
        return Err("index.html is missing next to the manifest".into());
    }
    // `{:?}` of a path string is a Rust string literal, backslashes escaped (Windows).
    let lit = |p: &Path| format!("{:?}", p.display().to_string());
    let mut files = vec![manifest.clone()];
    let mut bytes = |p: &Path| {
        files.push(p.to_path_buf());
        format!("include_bytes!({})", lit(p))
    };
    let mut s = String::from("/// **Private.** Generated by build.rs from the Vite manifest.\n");
    let _ = writeln!(
        s,
        "const EMBEDDED: Option<Bundle> = Some(Bundle {{\n    page: {},\n    assets: &[",
        bytes(&page)
    );
    for e in entries(&text, dist)? {
        let path = dist.join("assets").join(&e.name);
        let ext = e.name.rsplit_once('.').map_or("", |(_, x)| x);
        let identity = bytes(&path);
        let [br, gz] = ["br", "gz"].map(|z| {
            let sib = dist.join("assets").join(format!("{}.{z}", e.name));
            if sib.is_file() {
                format!("Some({})", bytes(&sib))
            } else {
                "None".into()
            }
        });
        let _ = writeln!(
            s,
            "        Asset {{ name: {:?}, content_type: {:?}, identity: {identity}, br: {br}, gz: {gz}, hash: {:?}, immutable: {} }},",
            e.name,
            content_type(ext),
            fnv1a(&read(&path)?),
            e.immutable
        );
    }
    s.push_str("    ],\n});\n");
    Ok(Some((s, files)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/studio-dist")
    }

    fn fixture_entries() -> Vec<Entry> {
        let dist = fixture();
        let text = std::fs::read_to_string(dist.join(".vite/manifest.json")).unwrap();
        entries(&text, &dist).unwrap()
    }

    #[test]
    fn only_allowlisted_referenced_files_are_in_the_table() {
        let names: Vec<_> = fixture_entries().into_iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            [
                "favicon.svg",
                "index-5UjPuW-k.css",
                "index-B7PI925R.js",
                "inter-latin-Dx4kXJAl.woff2"
            ]
        );
    }

    #[test]
    fn index_html_dotfiles_maps_and_the_manifest_are_never_in_the_table() {
        for e in fixture_entries() {
            assert!(!e.name.starts_with('.'), "{e:?}");
            for bad in ["index.html", ".map", "manifest.json", "notes.txt"] {
                assert!(!e.name.ends_with(bad), "{e:?}");
            }
        }
        // a file never referenced is never embedded, even with an allowlisted extension
        let dist = fixture();
        let only_css = r#"{"a.css":{"file":"assets/index-5UjPuW-k.css","src":"a.css"}}"#;
        let names: Vec<_> = entries(only_css, &dist)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["index-5UjPuW-k.css"]);
    }

    #[test]
    fn hashed_comes_from_the_manifest() {
        let got: Vec<_> = fixture_entries()
            .into_iter()
            .map(|e| (e.name, e.immutable))
            .collect();
        assert_eq!(
            got,
            [
                ("favicon.svg".to_string(), false),
                ("index-5UjPuW-k.css".to_string(), true),
                ("index-B7PI925R.js".to_string(), true),
                ("inter-latin-Dx4kXJAl.woff2".to_string(), true),
            ]
        );
        // only in an `assets` list: no source to compare with, so not hashed
        let m = r#"{"index.html":{"file":"assets/index-B7PI925R.js","src":"index.html",
                    "assets":["assets/inter-latin-Dx4kXJAl.woff2"]}}"#;
        let e = entries(m, &fixture()).unwrap();
        assert_eq!(
            e[1],
            Entry {
                name: "inter-latin-Dx4kXJAl.woff2".into(),
                immutable: false
            }
        );
    }

    /// `name -> immutable` for a manifest over the `studio-shapes` fixture files.
    fn shapes(manifest: &str) -> Vec<(String, bool)> {
        let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/studio-shapes");
        entries(manifest, &dist)
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.immutable))
            .collect()
    }

    fn one(manifest: &str, name: &str) -> bool {
        let all = shapes(manifest);
        all.iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} not in {all:?}"))
            .1
    }

    #[test]
    fn unhashed_names_are_never_immutable() {
        // a srcless chunk whose name is not hashed, with and without a `name`
        let m = r#"{"_react-flow.js":{"file":"assets/react-flow.js","name":"react-flow"}}"#;
        assert!(!one(m, "react-flow.js"));
        let m = r#"{"_react-flow.js":{"file":"assets/react-flow.js"}}"#;
        assert!(!one(m, "react-flow.js"), "no src, no name: no vote");
        // a fixed css file listed under a chunk
        let m = r#"{"index.html":{"file":"assets/index-DGbDado8.js","src":"index.html",
                    "css":["assets/style.css"]}}"#;
        assert!(!one(m, "style.css"));
        // a fixed `entryFileNames: assets/app.js` on src/main.tsx
        let m = r#"{"src/main.tsx":{"file":"assets/app.js","name":"main","src":"src/main.tsx"}}"#;
        assert!(!one(m, "app.js"));
        // an unhashed icon is no-cache; a hashed one immutable
        let m = r#"{"src/style.css":{"file":"assets/style.css","src":"src/style.css"},
                    "src/icon.svg":{"file":"assets/icon-Ab12Cd34.svg","src":"src/icon.svg"}}"#;
        assert!(!one(m, "style.css"));
        assert!(one(m, "icon-Ab12Cd34.svg"));
    }

    #[test]
    fn real_vite_shapes_are_immutable() {
        for (m, name) in [
            (
                r#"{"index.html":{"file":"assets/index-DGbDado8.js","name":"index","src":"index.html","isEntry":true}}"#,
                "index-DGbDado8.js",
            ),
            (
                r#"{"style.css":{"file":"assets/style-n-CS7iUc.css","src":"style.css"}}"#,
                "style-n-CS7iUc.css",
            ),
            (
                r#"{"src/panel/LogTab.tsx":{"file":"assets/LogTab-Bq3xYz12.js","name":"LogTab","src":"src/panel/LogTab.tsx"}}"#,
                "LogTab-Bq3xYz12.js",
            ),
            (
                r#"{"_react-flow-C5tso_Mv.js":{"file":"assets/react-flow-C5tso_Mv.js","name":"react-flow"}}"#,
                "react-flow-C5tso_Mv.js",
            ),
        ] {
            assert!(one(m, name), "{name}");
        }
    }

    #[test]
    fn the_real_build_sample_is_all_immutable() {
        let m = r#"{
          "_react-flow-C5tso_Mv.js": {"file": "assets/react-flow-C5tso_Mv.js", "name": "react-flow",
                                      "imports": ["_rolldown-runtime-WkwZ5CU-.js"]},
          "_rolldown-runtime-WkwZ5CU-.js": {"file": "assets/rolldown-runtime-WkwZ5CU-.js", "name": "rolldown-runtime"},
          "index.html": {"file": "assets/index-DGbDado8.js", "name": "index", "src": "index.html", "isEntry": true,
                         "imports": ["_react-flow-C5tso_Mv.js", "_rolldown-runtime-WkwZ5CU-.js"]},
          "style.css": {"file": "assets/style-n-CS7iUc.css", "src": "style.css"}
        }"#;
        assert_eq!(
            shapes(m),
            [
                ("index-DGbDado8.js".to_string(), true),
                ("react-flow-C5tso_Mv.js".to_string(), true),
                ("rolldown-runtime-WkwZ5CU-.js".to_string(), true),
                ("style-n-CS7iUc.css".to_string(), true),
            ]
        );
    }

    #[test]
    fn a_dot_segment_in_an_asset_path_is_an_error() {
        for file in ["../x.js", "assets/../x.js", "assets/./x.js"] {
            let m = format!(r#"{{"k":{{"file":"{file}","src":"k"}}}}"#);
            let e = entries(&m, &fixture()).unwrap_err();
            assert!(e.contains("segments"), "{file}: {e}");
        }
        // `.vite/` and dotfiles stay skipped
        let m = r#"{"k":{"file":"assets/.secret.js","assets":[".vite/x.js"]}}"#;
        assert_eq!(entries(m, &fixture()), Ok(Vec::new()));
    }

    #[test]
    fn a_missing_referenced_file_is_an_error() {
        let m = r#"{"x.js":{"file":"assets/gone-Ab12Cd34.js","src":"x.js"}}"#;
        let e = entries(m, &fixture()).unwrap_err();
        assert!(e.contains("gone-Ab12Cd34.js"), "{e}");
    }

    #[test]
    fn an_odd_layout_or_name_is_an_error() {
        for file in [
            "other/x-Ab12.js",
            "assets/sub/x-Ab12.js",
            "assets/x y.js",
            "assets/x%2e.js",
            "x.js",
        ] {
            let m = format!(r#"{{"k":{{"file":"{file}","src":"k"}}}}"#);
            assert!(entries(&m, &fixture()).is_err(), "{file}");
        }
        assert!(entries("[]", &fixture()).is_err(), "not an object");
        assert!(entries("{", &fixture()).is_err(), "not JSON");
    }

    #[test]
    fn generate_embeds_the_table_with_its_siblings() {
        let (code, files) = generate(&fixture()).unwrap().unwrap();
        // the manifest, the page, four assets, the js `.br` and `.gz`
        assert_eq!(files.len(), 8, "{files:?}");
        assert!(code.contains("const EMBEDDED: Option<Bundle>"), "{code}");
        assert!(code.contains(r#"name: "index-B7PI925R.js""#), "{code}");
        assert!(code.contains("immutable: false"), "favicon: {code}");
        assert_eq!(code.matches("br: Some(").count(), 1, "{code}");
        assert_eq!(code.matches("gz: Some(").count(), 1, "{code}");
        for bad in [".map", ".secret", "notes.txt", "manifest.json"] {
            assert!(!code.contains(bad), "{bad}: {code}");
        }
        let js = std::fs::read(fixture().join("assets/index-B7PI925R.js")).unwrap();
        assert!(code.contains(&format!("hash: \"{}\"", fnv1a(&js))));
    }

    #[test]
    fn generate_without_a_manifest_is_none() {
        let dir = fixture().join("assets");
        assert_eq!(generate(&dir), Ok(None));
    }

    #[test]
    fn content_types_and_hash() {
        assert_eq!(content_type("woff2"), "font/woff2");
        assert_eq!(fnv1a(b""), "cbf29ce484222325");
        assert_ne!(fnv1a(b"a"), fnv1a(b"b"));
    }
}
