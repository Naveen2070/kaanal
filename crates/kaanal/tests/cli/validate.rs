//! `kaanal validate`: valid, cyclic and malformed configs.

use crate::common::*;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[test]
fn validate_ok() {
    let out = bin()
        .args(["validate", "-c"])
        .arg(fixture())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn validate_cycle_fails_with_message() {
    let dir = scratch("cycle");
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n  b: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: a, to: b }\n  - { from: b, to: a }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cycle"), "{}", stderr(&out));
}

#[test]
fn validate_bad_yaml_shows_file_line_col() {
    let dir = scratch("yaml");
    let file = dir.join("kaanal.yaml");
    std::fs::write(&file, "version: 1\nport: nope\nnodes: {}\n").unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("kaanal.yaml:2:"), "{err}");
}

#[test]
fn validate_prints_unsupported_keyword_warnings() {
    let dir = scratch("warn");
    std::fs::copy(
        fixtures().join("openapi-warn.yaml"),
        dir.join("openapi-warn.yaml"),
    )
    .unwrap();
    let file = dir.join("kaanal.yaml");
    std::fs::write(
        &file,
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi-warn.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let out = bin().args(["validate", "-c"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("warning: ") && err.contains("'not'"), "{err}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok: "));
}

/// A one-service project whose spec pins `total: 99` over `maximum: 5`, so every sample misses.
fn bad_response_project(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(
        dir.join("openapi.yaml"),
        r#"openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /orders:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  total: { type: integer, maximum: 5, example: 99 }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    dir
}

#[test]
fn validate_reports_generated_responses_that_break_their_schema() {
    let dir = bad_response_project("selfcheck-validate");
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let e = stderr(&out);
    assert!(
        e.contains(
            "GET /orders: 5 of 5 generated samples violate the response schema: /total: maximum"
        ),
        "{e}"
    );
}

#[test]
fn up_serves_first_then_reports_schema_violations() {
    let dir = bad_response_project("selfcheck-up");
    let log = dir.join("up.err");
    let child = bin()
        .args(["up", "--port", "0", "-c", "kaanal.yaml"])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let guard = Helper(child);
    let run = dir.join(".kaanal/run.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !run.exists() {
        assert!(Instant::now() < deadline, "up never became ready");
        std::thread::sleep(Duration::from_millis(50));
    }
    // the server is up and answers before the check has to be done
    let port = {
        let text = std::fs::read_to_string(&run).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["port"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(health(u16::try_from(port).unwrap()), 200);
    let text = loop {
        let t = std::fs::read_to_string(&log).unwrap_or_default();
        if t.contains("violate the response schema") {
            break t;
        }
        assert!(Instant::now() < deadline, "no self-check line: {t}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(text.contains("GET /orders:"), "{text}");
    drop(guard);
}
