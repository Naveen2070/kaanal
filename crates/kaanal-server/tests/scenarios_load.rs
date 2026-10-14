use kaanal_core::{Config, Located};
use std::ops::Deref;
use std::path::{Path, PathBuf};

const BASE: &str = "version: 1
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";
const GOOD: &str = "chaos:\n  - { node: db, down: true }\n";

/// A temp dir removed on drop, even when the test fails.
struct Dir(PathBuf);
impl Deref for Dir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh project dir holding the spec and a `scenarios/` dir (when `files` is not empty).
fn project(name: &str, files: &[(&str, &[u8])]) -> Dir {
    let d = std::env::temp_dir().join(format!("kaanal-sc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, d.join("openapi.yaml")).unwrap();
    let d = Dir(d);
    for (f, body) in files {
        let p = d.join("scenarios").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    d
}

fn check(dir: &Path, extra: &str) -> Result<Vec<String>, Located> {
    let cfg = Config::from_yaml(&format!("{BASE}{extra}")).unwrap();
    kaanal_server::check(&cfg, dir)
}

/// What the CLI prints: the file (if any) and the message.
fn shown(e: &Located) -> String {
    format!(
        "{} {}",
        e.file
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_default(),
        e.msg
    )
}

#[test]
fn absent_dir_is_fine() {
    let d = project("absent", &[]);
    assert_eq!(check(&d, "").unwrap(), Vec::<String>::new());
}

#[test]
fn discovery_takes_only_plain_yaml_files() {
    // Everything but ok.yaml and bom.yaml must be ignored, so each is invalid on purpose.
    let junk: &[u8] = b"chaos: [ { node: ghost, down: true } ]\n";
    let bom = "\u{feff}chaos: [ { node: db, error_kind: reset } ]\n";
    let d = project(
        "discover",
        &[
            ("ok.yaml", GOOD.as_bytes()),
            ("bom.yaml", bom.as_bytes()),
            ("sub/x.yaml", junk),
            (".hidden.yaml", junk),
            ("x.yaml~", junk),
            (".#lock.yaml", junk),
            ("notes.txt", junk),
        ],
    );
    let w = check(&d, "").unwrap();
    // The BOM file loaded (its orphan error_kind is the only warning).
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("bom"), "{w:?}");
    assert!(w[0].contains("error_rate"), "{w:?}");
}

#[test]
fn invalid_stem_names_the_file() {
    let d = project("stem", &[("Bad.yaml", GOOD.as_bytes())]);
    let e = check(&d, "").unwrap_err();
    assert!(shown(&e).contains("Bad.yaml"), "{e}");
    assert!(e.msg.contains("invalid scenario name"), "{e}");
}

#[test]
fn oversized_file_is_refused() {
    let big = format!("description: {}\nchaos: []\n", "x".repeat(1 << 20));
    let d = project("big", &[("big.yaml", big.as_bytes())]);
    let e = check(&d, "").unwrap_err();
    assert!(shown(&e).contains("big.yaml"), "{e}");
    assert!(e.msg.contains("1 MiB"), "{e}");
}

#[test]
fn too_many_files_are_refused() {
    let files: Vec<(String, &[u8])> = (0..129)
        .map(|i| (format!("s{i}.yaml"), GOOD.as_bytes()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), *b)).collect();
    let d = project("many", &refs);
    let e = check(&d, "").unwrap_err();
    assert!(e.msg.contains("at most 128"), "{e}");
}

#[test]
fn inline_and_file_with_one_name_clash() {
    let d = project("clash", &[("pd.yaml", GOOD.as_bytes())]);
    let e = check(
        &d,
        "scenarios:\n  pd:\n    chaos: [ { node: db, down: true } ]\n",
    )
    .unwrap_err();
    assert!(e.msg.contains("both the config and a scenario file"), "{e}");
}

#[test]
fn file_errors_point_into_the_file() {
    let text =
        "description: x\nchaos:\n  - { node: db, down: true }\n  - { node: ghost, down: true }\n";
    let d = project("filepos", &[("ok.yaml", text.as_bytes())]);
    let e = check(&d, "").unwrap_err();
    assert!(
        e.file.as_ref().is_some_and(|f| f.ends_with("ok.yaml")),
        "{e:?}"
    );
    assert!(e.msg.contains("unknown node 'ghost'"), "{e}");
    let at = e.in_source(text);
    assert_eq!((at.line, at.col), (Some(4), Some(13)), "{at}");
}

#[test]
fn file_parse_errors_carry_their_position() {
    let d = project("fileparse", &[("ok.yaml", b"chaos: []\nbogus: 1\n")]);
    let e = check(&d, "").unwrap_err();
    assert!(
        e.file.as_ref().is_some_and(|f| f.ends_with("ok.yaml")),
        "{e:?}"
    );
    assert_eq!(e.pos.map(|p| p.0), Some(2), "{e:?}");
}

#[test]
fn inline_errors_point_into_kaanal_yaml() {
    let d = project("inline", &[]);
    let extra = "scenarios:\n  pd:\n    chaos:\n      - { node: db, down: true }\n      - { node: db, error_rate: 2 }\n";
    let src = format!("{BASE}{extra}");
    let e = check(&d, extra).unwrap_err();
    assert_eq!(e.file, None);
    assert!(e.msg.contains("error_rate"), "{e}");
    let at = e.in_source(&src);
    // Value of `error_rate` on the last line.
    assert_eq!(at.line, Some(src.lines().count()), "{at}");
}

#[test]
fn inline_endpoint_checks_and_warnings() {
    let d = project("inline-warn", &[]);
    let w = check(
        &d,
        "scenarios:\n  pd:\n    chaos:\n      - { endpoint: \"get /users/{id}\", error_kind: hang }\n",
    )
    .unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("pd"), "{w:?}");
    let e = check(
        &d,
        "scenarios:\n  pd:\n    chaos:\n      - { endpoint: \"GET /nope\", down: true }\n",
    )
    .unwrap_err();
    assert!(e.msg.contains("unknown endpoint"), "{e}");
}

#[test]
fn bad_inline_name_is_rejected() {
    let d = project("inline-name", &[]);
    let e = check(&d, "scenarios:\n  Active:\n    chaos: []\n").unwrap_err();
    assert!(e.msg.contains("invalid scenario name"), "{e}");
}

/// Creates a symlink; false when the platform or privileges do not allow it.
fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    } else {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    };
    #[cfg(not(any(unix, windows)))]
    return false;
}

const ORPHAN: &str = "chaos: [ { node: db, error_kind: reset } ]\n";

#[test]
fn a_symlinked_scenarios_dir_is_followed() {
    let d = project("linkdir", &[("real/x.yaml", ORPHAN.as_bytes())]);
    if !symlink(&d.join("scenarios/real"), &d.join("scenarios-link")) {
        return; // no permission to create symlinks here
    }
    std::fs::rename(d.join("scenarios"), d.join("target")).unwrap();
    // `scenarios` now a link to the directory that holds x.yaml.
    if !symlink(&d.join("target/real"), &d.join("scenarios")) {
        return;
    }
    let w = check(&d, "").unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("'x'"), "{w:?}");
}

#[test]
fn symlinked_files_are_skipped_with_a_warning() {
    let d = project("linkfile", &[("ok.yaml", GOOD.as_bytes())]);
    std::fs::write(d.join("real.txt"), ORPHAN).unwrap();
    if !symlink(&d.join("real.txt"), &d.join("scenarios/link.yaml")) {
        return;
    }
    let w = check(&d, "").unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("skipped symlink scenarios/link.yaml"),
        "{w:?}"
    );
}

#[test]
fn near_miss_extensions_warn() {
    let junk: &[u8] = b"chaos: [ { node: ghost, down: true } ]\n";
    let d = project(
        "ext",
        &[("a.yml", junk), ("B.YAML", junk), ("c.Yaml", junk)],
    );
    let w = check(&d, "").unwrap();
    assert_eq!(w.len(), 3, "{w:?}");
    assert!(w.iter().all(|m| m.contains("must end in .yaml")), "{w:?}");
}

#[cfg(unix)]
#[test]
fn non_utf8_names_warn() {
    use std::os::unix::ffi::OsStrExt as _;
    let d = project("nonutf8", &[("ok.yaml", GOOD.as_bytes())]);
    let name = std::ffi::OsStr::from_bytes(b"bad\xff.yaml");
    std::fs::write(d.join("scenarios").join(name), GOOD).unwrap();
    let w = check(&d, "").unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("not UTF-8"), "{w:?}");
}

#[test]
fn non_utf8_content_is_an_error_in_the_file() {
    let d = project("badbytes", &[("ok.yaml", b"chaos: []\n# \xff\xfe\n")]);
    let e = check(&d, "").unwrap_err();
    assert!(e.msg.contains("UTF-8"), "{e}");
    assert!(shown(&e).contains("ok.yaml"), "{e}");
}

#[test]
fn the_size_limit_is_exactly_one_mib() {
    let head = GOOD.to_string() + "#";
    let at = |n: usize| head.clone() + &"x".repeat(n - head.len());
    let d = project("exact", &[("ok.yaml", at(1 << 20).as_bytes())]);
    check(&d, "").unwrap();
    let d = project("over", &[("ok.yaml", at((1 << 20) + 1).as_bytes())]);
    let e = check(&d, "").unwrap_err();
    assert!(e.msg.contains("1 MiB"), "{e}");
}
