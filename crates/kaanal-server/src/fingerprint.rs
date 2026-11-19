//! The system's design fingerprint: 12 hex digits over a canonical digest of every input the
//! build reads, so a sweep report can say whether two sweeps ran on the same design. The inputs
//! are the config, each node's `spec:` file, the `endpoints:` file, `scenarios/*.yaml`,
//! `locales/*.yaml` and the `data:` file or `data.yaml`, found by the loaders' own rules. The
//! `site:` folder is not an input. A false "same design" is the failure that matters; a false
//! "different" is harmless.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kaanal_core::Config;
use kaanal_core::config::{EndpointsConfig, NodeConfig};
use kaanal_core::fingerprint::{canonical_digest, combine};
use kaanal_data::DataSet;

use crate::generation::{Trust, scenarios_dir};
use crate::paths::{read_text, resolve};
use crate::scenarios;

/// **Private.** The digest of an input that cannot be read or listed now, though the build read it
/// a moment ago (deleted or changed in between). Its label still counts, so the fingerprint
/// differs from that of a project without the input; all zeros is no SHA-256 anyone will meet.
const UNREADABLE: [u8; 32] = [0; 32];

/// The fingerprint of the system `cfg` builds from `base_dir`: [`combine`] over [`parts`]. Never
/// fails (see [`UNREADABLE`]). **Known and accepted:** the files are read again here, right after
/// the loaders and from the same `base_dir`; only the config's digest is of the exact bytes parsed.
pub(crate) fn of_system(cfg: &Config, base_dir: &Path) -> String {
    combine(&parts(cfg, base_dir))
}

/// **Private.** One `(label, digest)` per input: `config` for the config (its parsed text), else
/// the file's path relative to `base_dir` with forward slashes, its text through
/// [`canonical_digest`]. Each file is read once (two nodes naming one spec count it once); a
/// missing optional input adds nothing.
fn parts(cfg: &Config, base_dir: &Path) -> Vec<(String, [u8; 32])> {
    // label -> file; `None`: a listing that failed now
    let mut files: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    let mut add = |rel: &str| {
        // the owner's rule never fails: it is `base_dir` joined with `rel`
        if let Ok(f) = resolve(base_dir, rel, Trust::Owner, &[]) {
            files.insert(label(base_dir, &f), Some(f));
        }
    };
    for spec in cfg.nodes.values().filter_map(NodeConfig::spec) {
        add(spec);
    }
    if let Some(EndpointsConfig::File(p)) = &cfg.endpoints {
        add(p);
    }
    match scenarios::list(&scenarios_dir(base_dir)) {
        Ok((found, _)) => files.extend(
            found
                .into_iter()
                .map(|(stem, f)| (format!("scenarios/{stem}.yaml"), Some(f))),
        ),
        Err(_) => drop(files.insert("scenarios/".into(), None)),
    }
    match DataSet::files(base_dir, cfg.data.as_deref()) {
        Ok(found) => files.extend(found.into_iter().map(|f| (label(base_dir, &f), Some(f)))),
        Err(_) => drop(files.insert("locales/".into(), None)),
    }
    let mut out = vec![("config".to_string(), cfg.digest)];
    out.extend(files.into_iter().map(|(l, f)| {
        let text = f.and_then(|f| read_text(&f).ok());
        (l, text.map_or(UNREADABLE, |t| canonical_digest(&t)))
    }));
    out
}

/// **Private.** `file` relative to `base_dir`, forward slashes; as is when it is not under it.
fn label(base_dir: &Path, file: &Path) -> String {
    file.strip_prefix(base_dir)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{Generation, Trust};
    use kaanal_core::Config;
    use std::path::{Path, PathBuf};

    const SPEC: &str = "openapi: 3.0.0\ninfo: { title: t, version: '1' }\npaths:\n  /ping:\n    get:\n      responses:\n        '200':\n          description: ok\n          content: { application/json: { schema: { type: object, properties: { a: { type: string } } } } }\n";
    /// Every kind of input, plus a `site:` folder (not an input).
    const FULL: &str = "version: 1\nseed: 42\nendpoints: endpoints.yaml\nsite: site\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: api.yaml }\nedges:\n  - { from: gw, to: app }\n";
    /// The required inputs only: no endpoints file, `scenarios/`, `locales/` or `data.yaml`.
    const BARE: &str = "version: 1\nseed: 42\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: api.yaml }\nedges:\n  - { from: gw, to: app }\n";
    const ENDPOINTS: &str = "\"GET /ping\": { override: { body: { ok: true } } }\n";
    const SCENARIO: &str = "chaos:\n  - { node: app, latency_ms: 30 }\n";
    const PACK: &str = "version: 1\ntag: xx-YY\nname_order: given-family\ncountry: Nowhere\ncurrency: { code: TST, symbol: T }\nphone: { prefix: \"+0\", formats: [\"{4}\"] }\naddress: \"{number} {street}, {city}\"\nfirst_names: [Ann]\nlast_names: [Lee]\nstreets: [Main]\nplaces: [{ city: Town, postcode: \"{5}\" }]\n";
    const DATA: &str =
        "fields:\n  sku: { gen: static, value: X }\n  tag: { gen: static, value: T }\n";

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A fresh project folder: `full` has every kind of input, else only the config and spec.
    fn project(name: &str, full: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kaanal-fp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "kaanal.yaml", if full { FULL } else { BARE });
        write(&dir, "api.yaml", SPEC);
        if full {
            write(&dir, "endpoints.yaml", ENDPOINTS);
            write(&dir, "scenarios/slow.yaml", SCENARIO);
            write(&dir, "locales/xx-YY.yaml", PACK);
            write(&dir, "data.yaml", DATA);
            write(&dir, "site/index.html", "<p>hi</p>\n");
        }
        dir
    }

    fn config(dir: &Path) -> Config {
        Config::from_yaml(&std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap()).unwrap()
    }

    fn fp(dir: &Path) -> String {
        of_system(&config(dir), dir)
    }

    fn labels(dir: &Path) -> Vec<String> {
        let mut l: Vec<String> = parts(&config(dir), dir).into_iter().map(|p| p.0).collect();
        l.sort();
        l
    }

    /// Replaces `from` (which must be there) with `to` in the file `rel`.
    fn edit(dir: &Path, rel: &str, from: &str, to: &str) {
        let text = std::fs::read_to_string(dir.join(rel)).unwrap();
        assert!(text.contains(from), "{rel} has no {from:?}");
        std::fs::write(dir.join(rel), text.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn a_comment_or_key_order_change_in_any_input_keeps_the_fingerprint() {
        let dir = project("cosmetic", true);
        let before = fp(&dir);
        assert_eq!(before.len(), 12);
        assert!(before.bytes().all(|b| b.is_ascii_hexdigit()));
        for rel in [
            "kaanal.yaml",
            "api.yaml",
            "endpoints.yaml",
            "scenarios/slow.yaml",
            "locales/xx-YY.yaml",
            "data.yaml",
        ] {
            let text = std::fs::read_to_string(dir.join(rel)).unwrap();
            std::fs::write(dir.join(rel), format!("# a comment\n{text}\n\n# the end\n")).unwrap();
        }
        // key order: the config's `seed:` moves to the end, the dictionary's fields swap
        edit(&dir, "kaanal.yaml", "seed: 42\n", "");
        let text = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
        std::fs::write(dir.join("kaanal.yaml"), format!("{text}seed: 42\n")).unwrap();
        edit(
            &dir,
            "data.yaml",
            "  sku: { gen: static, value: X }\n  tag: { gen: static, value: T }\n",
            "  tag: { value: T, gen: static }\n  sku: { value: X, gen: static }\n",
        );
        assert_eq!(fp(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_kind_of_input_file_changes_the_fingerprint() {
        for (rel, from, to) in [
            ("kaanal.yaml", "seed: 42", "seed: 43"),
            (
                "api.yaml",
                "a: { type: string }",
                "a: { type: string }, b: { type: integer }",
            ),
            ("endpoints.yaml", "ok: true", "ok: false"),
            ("scenarios/slow.yaml", "latency_ms: 30", "latency_ms: 40"),
            (
                "locales/xx-YY.yaml",
                "country: Nowhere",
                "country: Elsewhere",
            ),
            ("data.yaml", "value: X", "value: Y"),
        ] {
            let dir = project("kinds", true);
            let before = fp(&dir);
            edit(&dir, rel, from, to);
            assert_ne!(fp(&dir), before, "editing {rel} kept the fingerprint");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_configured_data_file_is_counted_in_place_of_data_yaml() {
        let dir = project("data-key", true);
        edit(
            &dir,
            "kaanal.yaml",
            "seed: 42\n",
            "seed: 42\ndata: conf/mine.yaml\n",
        );
        write(&dir, "conf/mine.yaml", DATA);
        let l = labels(&dir);
        assert!(l.contains(&"conf/mine.yaml".to_string()), "{l:?}");
        assert!(!l.contains(&"data.yaml".to_string()), "{l:?}");
        let before = fp(&dir);
        edit(&dir, "conf/mine.yaml", "value: X", "value: Y");
        assert_ne!(fp(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_without_optional_inputs_has_a_stable_fingerprint() {
        let dir = project("bare", false);
        assert_eq!(labels(&dir), ["api.yaml", "config"]);
        let first = fp(&dir);
        assert_eq!(fp(&dir), first, "two calls agree");
        std::fs::create_dir_all(dir.join("scenarios")).unwrap();
        assert_eq!(fp(&dir), first, "an empty scenarios/ adds nothing");
        std::fs::remove_dir_all(dir.join("scenarios")).unwrap();
        assert_eq!(fp(&dir), first, "and removing it again changes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_input_that_cannot_be_read_now_still_gives_a_fingerprint() {
        let dir = project("gone", true);
        let before = fp(&dir);
        // the config still names it, but it is gone (a moment ago the build read it)
        std::fs::remove_file(dir.join("endpoints.yaml")).unwrap();
        let after = fp(&dir);
        assert_eq!(after.len(), 12);
        assert_ne!(after, before);
        assert!(labels(&dir).contains(&"endpoints.yaml".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_generation_carries_it() {
        let dir = project("gen", true);
        let cfg = config(&dir);
        let (g, _) = Generation::build(&cfg, &dir, None, Trust::Owner).unwrap();
        assert_eq!(g.fingerprint, of_system(&cfg, &dir));
        let (again, _) = Generation::build(&cfg, &dir, None, Trust::Owner).unwrap();
        assert_eq!(g.fingerprint, again.fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every file under `dir`, relative to it, forward slashes.
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir.join(rel)).unwrap() {
            let name = e.unwrap().file_name().into_string().unwrap();
            let r = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if dir.join(&r).is_dir() {
                walk(dir, &r, out);
            } else {
                out.push(r);
            }
        }
    }

    /// The drift guard. A file the build reads is one whose corruption fails the build: corrupt
    /// every file of a project that has every kind of input (and a `site:` folder, which is not
    /// one), and the files that fail the build must be exactly the counted parts.
    #[test]
    fn every_loader_input_is_counted() {
        let dir = project("drift", true);
        assert_eq!(
            labels(&dir),
            [
                "api.yaml",
                "config",
                "data.yaml",
                "endpoints.yaml",
                "locales/xx-YY.yaml",
                "scenarios/slow.yaml",
            ]
        );
        let cfg = config(&dir);
        assert!(Generation::build(&cfg, &dir, None, Trust::Owner).is_ok());
        let mut files = Vec::new();
        walk(&dir, "", &mut files);
        files.sort();
        let mut read_by_build = vec!["config".to_string()];
        for rel in files.iter().filter(|f| *f != "kaanal.yaml") {
            let text = std::fs::read_to_string(dir.join(rel)).unwrap();
            std::fs::write(dir.join(rel), "{{{ not: [valid\n").unwrap();
            if Generation::build(&cfg, &dir, None, Trust::Owner).is_err() {
                read_by_build.push(rel.clone());
            }
            std::fs::write(dir.join(rel), text).unwrap();
        }
        read_by_build.sort();
        assert!(files.contains(&"site/index.html".to_string()), "{files:?}");
        assert_eq!(
            labels(&dir),
            read_by_build,
            "the build reads a file the fingerprint does not count, or the other way round"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
