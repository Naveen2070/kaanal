//! Whole-project checks: the sample project, the schema, the seed line, dictionaries and locales.

use crate::common::*;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

#[test]
fn sample_project_validates() {
    // The shipped example differs from the test fixture (lognormal costs, port 8080), so check it too.
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nginx-server/kaanal.yaml");
    let out = bin()
        .args(["validate", "-c"])
        .arg(&sample)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "the sample should have no warnings: {}",
        stderr(&out)
    );
}

#[test]
fn validate_places_errors_in_later_nodes_and_edges() {
    let dir = scratch("positions");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1
nodes:
  app: { type: service }
  db: { type: database, flavor: postgres, bogus: 1 }
",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("kaanal.yaml:4:") && err.contains("node 'db': unknown field 'bogus'"),
        "{err}"
    );
    std::fs::write(
        &file,
        "version: 1
nodes:
  a: { type: service }
  b: { type: service }
edges:
  - { from: a, to: b }
  - { from: b, to: a }
",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert!(
        stderr(&out).contains("kaanal.yaml:7:5: cycle: a -> b -> a"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn schema_prints_the_committed_schema() {
    let out = bin().arg("schema").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-core/kaanal.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(printed, committed);
}

#[test]
fn gatewayless_example_validates() {
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/gatewayless/kaanal.yaml");
    let out = bin()
        .args(["validate", "-c"])
        .arg(&sample)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no warnings expected: {}",
        stderr(&out)
    );
}

#[test]
fn up_prints_the_realistic_seed() {
    let dir = scratch("realistic");
    std::fs::copy(fixtures().join("openapi.yaml"), dir.join("openapi.yaml")).unwrap();
    let src = std::fs::read_to_string(fixture())
        .unwrap()
        .replace("seed: 42", "seed: 42\ndeterminism: realistic");
    std::fs::write(dir.join("kaanal.yaml"), src).unwrap();
    let mut child = bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    let line = BufReader::new(out)
        .lines()
        .map(Result::unwrap)
        .find(|l| l.starts_with("Seed:"));
    let _ = child.kill();
    let _ = child.wait();
    let line = line.expect("up prints a Seed: line under determinism: realistic");
    let n: u64 = line["Seed:".len()..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        line,
        format!(
            "Seed:  {n} (determinism: realistic; replay its timings and failures with determinism: controlled and seed: {n}; generated data then follows that seed too)"
        )
    );
}

#[test]
fn validate_rejects_routes_with_a_pointer() {
    let dir = scratch("routes");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  app: { type: service }\nroutes:\n  \"GET /health\": { body: 1 }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.yaml:4:1: routes: was replaced by endpoints: (see docs)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn validate_reports_a_bad_dictionary_entry_with_line_and_column() {
    let dir = project("bad-dictionary", "");
    std::fs::write(
        dir.join("data.yaml"),
        "version: 1\nfields:\n  sku: { gen: sku_code }\n",
    )
    .unwrap();
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let e = stderr(&out);
    assert!(e.contains("data.yaml:3:"), "{e}");
    assert!(e.contains("sku_code") && e.contains("first_name"), "{e}");
}

#[test]
fn validate_accepts_the_ja_jp_fixture_project_and_rejects_an_unknown_locale() {
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let out = bin()
        .args(["validate", "-c"])
        .arg(fx.join("kaanal.yaml"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let dir = project("unknown-locale", "locale: ja-JP\n");
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(1));
    let e = stderr(&out);
    assert!(
        e.contains("kaanal.yaml:9:9: ")
            && e.contains("de-DE, en-GB, en-IN, en-US, es-ES, fr-FR, pt-BR"),
        "{e}"
    );
}

#[test]
fn schema_prints_config_dictionary_or_pack() {
    let plain = bin().arg("schema").output().unwrap();
    let config = bin().args(["schema", "config"]).output().unwrap();
    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(
        plain.stdout, config.stdout,
        "`kaanal schema` stays the config schema"
    );
    for (which, prop) in [("dictionary", "fields"), ("pack", "first_names")] {
        let out = bin().args(["schema", which]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{which}: {}", stderr(&out));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v["properties"][prop].is_object(), "{which}: {v}");
    }
    let out = bin().args(["schema", "nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64));
}
