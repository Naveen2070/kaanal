//! The system's design fingerprint: 12 hex digits over a canonical digest of every input the
//! build reads, so a sweep report can say whether two sweeps ran on the same design. The inputs
//! are the config, each node's `spec:` file, the `endpoints:` file, `scenarios/*.yaml`,
//! `locales/*.yaml` and the `data:` file or `data.yaml`. The `site:` folder is not an input. A
//! false "same design" is the failure that matters; a false "different" is harmless. It covers the
//! design files only: the chaos a sweep ran under is recorded beside it (RULES 140).
//!
//! The loaders hand each input over as they read it ([`Inputs`]): the text, or for a spec the
//! document the loader parsed already, so no file is read twice (no window for an edit between
//! the build and the fingerprint) and a large spec is not parsed twice (measured 2026-11-21: the
//! second read and parse cost more than the rest of the build on 50 MiB of specs).

use std::collections::BTreeMap;
use std::path::Path;

use kaanal_core::fingerprint::{canonical_digest, combine, value_digest};

use crate::generation::Trust;
use crate::paths::resolve;

/// The design inputs one build read: label to digest. The label is the file's path relative to
/// the project folder with forward slashes (`scenarios/NAME.yaml` for a scenario file), so a file
/// read twice (two nodes naming one spec) counts once.
#[derive(Default)]
pub(crate) struct Inputs(BTreeMap<String, [u8; 32]>);

impl Inputs {
    /// An input read as `text`, through [`canonical_digest`].
    pub(crate) fn text(&mut self, label: String, text: &str) {
        self.0.insert(label, canonical_digest(text));
    }

    /// An input the loader parsed to `doc` already: [`value_digest`], the digest of its text
    /// (except past the alias bound, see `value_digest`: only ever a false "different").
    pub(crate) fn parsed(&mut self, label: String, doc: &serde_json::Value) {
        self.0.insert(label, value_digest(doc));
    }

    /// The fingerprint: [`combine`] over `config` (the config text's digest) and every input.
    pub(crate) fn fingerprint(&self, config: [u8; 32]) -> String {
        let mut parts = vec![("config".to_string(), config)];
        parts.extend(self.0.iter().map(|(l, d)| (l.clone(), *d)));
        combine(&parts)
    }

    /// The labels counted, `config` included, sorted.
    #[cfg(test)]
    pub(crate) fn labels(&self) -> Vec<String> {
        let mut l: Vec<String> = self.0.keys().cloned().collect();
        l.push("config".into());
        l.sort();
        l
    }
}

/// The label of `rel` as the config names it: resolved by the owner's rule (`base_dir` joined
/// with `rel`), so a draft build (held to the project by [`Trust::Api`]) labels a file as a boot does.
pub(crate) fn rel_label(base_dir: &Path, rel: &str) -> String {
    // the owner's rule never fails: it is `base_dir` joined with `rel`
    resolve(base_dir, rel, Trust::Owner, &[])
        .map_or_else(|_| rel.replace('\\', "/"), |f| label(base_dir, &f))
}

/// `file` relative to `base_dir`, forward slashes; as is when it is not under it.
pub(crate) fn label(base_dir: &Path, file: &Path) -> String {
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
    use std::path::PathBuf;

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

    /// The oracle: the files listed and read again from disk, by the listing helpers the loaders
    /// share (`scenarios::list`, `DataSet::files`), the way the fingerprint was taken before the
    /// loaders handed their inputs over. The build's fingerprint must equal it on a quiet project.
    fn reread(cfg: &Config, base_dir: &Path) -> (String, Vec<String>) {
        use crate::paths::read_text;
        use kaanal_core::config::{EndpointsConfig, NodeConfig};
        let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
        for spec in cfg.nodes.values().filter_map(NodeConfig::spec) {
            let f = resolve(base_dir, spec, Trust::Owner, &[]).unwrap();
            files.insert(label(base_dir, &f), f);
        }
        if let Some(EndpointsConfig::File(p)) = &cfg.endpoints {
            let f = resolve(base_dir, p, Trust::Owner, &[]).unwrap();
            files.insert(label(base_dir, &f), f);
        }
        let (found, _) =
            crate::scenarios::list(&crate::generation::scenarios_dir(base_dir)).unwrap();
        files.extend(
            found
                .into_iter()
                .map(|(stem, f)| (format!("scenarios/{stem}.yaml"), f)),
        );
        let data = kaanal_data::DataSet::files(base_dir, cfg.data.as_deref()).unwrap();
        files.extend(data.into_iter().map(|f| (label(base_dir, &f), f)));
        let mut parts = vec![("config".to_string(), cfg.digest)];
        parts.extend(
            files
                .iter()
                .map(|(l, f)| (l.clone(), canonical_digest(&read_text(f).unwrap()))),
        );
        let mut labels: Vec<String> = parts.iter().map(|p| p.0.clone()).collect();
        labels.sort();
        (combine(&parts), labels)
    }

    /// The fingerprint a build of `dir` takes.
    fn fp(dir: &Path) -> String {
        Generation::build(&config(dir), dir, None, Trust::Owner)
            .unwrap()
            .0
            .fingerprint
    }

    /// The inputs a build of `dir` counted.
    fn labels(dir: &Path) -> Vec<String> {
        Generation::build_counted(&config(dir), dir, None, Trust::Owner)
            .unwrap()
            .2
            .labels()
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
    fn the_build_fingerprint_equals_a_fresh_reading_of_its_files() {
        // the oracle reads the files again; on a quiet project both must agree, label for label
        for (name, full) in [("oracle-full", true), ("oracle-bare", false)] {
            let dir = project(name, full);
            let cfg = config(&dir);
            let (g, _, inputs) = Generation::build_counted(&cfg, &dir, None, Trust::Owner).unwrap();
            let (want, want_labels) = reread(&cfg, &dir);
            assert_eq!(g.fingerprint, want, "{name}");
            assert_eq!(inputs.labels(), want_labels, "{name}");
            let (again, _) = Generation::build(&cfg, &dir, None, Trust::Owner).unwrap();
            assert_eq!(g.fingerprint, again.fingerprint, "two builds agree");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_draft_build_labels_its_inputs_as_a_boot_does() {
        let dir = project("draft-labels", true);
        let cfg = config(&dir);
        let owner = Generation::build_counted(&cfg, &dir, None, Trust::Owner).unwrap();
        let api = Generation::build_counted(&cfg, &dir, None, Trust::Api).unwrap();
        assert_eq!(owner.2.labels(), api.2.labels());
        assert_eq!(owner.0.fingerprint, api.0.fingerprint);
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
