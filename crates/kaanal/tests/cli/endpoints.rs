//! `kaanal endpoints` and the endpoints file.

use crate::common::*;

/// What `kaanal endpoints` adds when only `GET /users` has an entry (D22).
pub(crate) const SKELETON: &str = "# Added by kaanal endpoints\n\"GET /users/{id}\": {} # owner app, op read\n\"POST /orders\": {} # owner app, op write\n";

#[test]
fn endpoints_command_prints_and_appends_only() {
    let dir = project("eps-append", "endpoints: kaanal.endpoints.yaml\n");
    let file = dir.join("kaanal.endpoints.yaml");
    let old = "# my notes\r\n\"GET /users\": { override: { body: [] } } # keep me\r\n";
    std::fs::write(&file, old).unwrap();
    let out = kaanal(&dir, &["endpoints"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), SKELETON);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        old,
        "printing never writes"
    );
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let appended = format!("{old}{}", SKELETON.replace('\n', "\r\n"));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        appended,
        "old text untouched, CRLF kept"
    );
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert!(stdout(&out).contains("nothing to add"), "{}", stdout(&out));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), appended);
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no stale warnings: {}",
        stderr(&out)
    );
}

#[test]
fn endpoints_write_refuses_inline_and_existing_files() {
    // Inline map: print only; kaanal.yaml is never rewritten (R1.17).
    let dir = project("eps-inline", "endpoints:\n  \"GET /users\": {}\n");
    let before = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Indented for pasting below `endpoints:`.
    let indented: String = SKELETON.lines().map(|l| ["  ", l, "\n"].concat()).collect();
    assert_eq!(stdout(&out), indented);
    assert!(
        stderr(&out).contains("endpoints: is inline"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        before
    );
    // ...and pasting it as printed gives a config that validates with nothing stale.
    std::fs::write(dir.join("kaanal.yaml"), format!("{before}{}", stdout(&out))).unwrap();
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    std::fs::write(dir.join("kaanal.yaml"), &before).unwrap();
    // No `endpoints:` key: --write creates kaanal.endpoints.yaml and says how to wire it.
    let dir = project("eps-new", "");
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let created = std::fs::read_to_string(dir.join("kaanal.endpoints.yaml")).unwrap();
    assert_eq!(
        created,
        "# Added by kaanal endpoints\n\"GET /users\": {} # owner app, op scan\n\"GET /users/{id}\": {} # owner app, op read\n\"POST /orders\": {} # owner app, op write\n"
    );
    assert!(
        stdout(&out).contains("endpoints: kaanal.endpoints.yaml"),
        "{}",
        stdout(&out)
    );
    // ...and refuses to touch that file while kaanal.yaml still does not name it.
    let out = kaanal(&dir, &["endpoints", "--write"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.endpoints.yaml exists"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.endpoints.yaml")).unwrap(),
        created
    );
}

#[test]
fn validate_warns_about_stale_entries_and_ignored_error_maps() {
    let dir = project(
        "eps-warn",
        "endpoints:\n  \"GET /old\": {}\n  \"GET /health\": { override: { body: 1 }, chain: [ { call: app, via: \"POST /orders\" } ] }\n  \"POST /orders\": { error_map: { \"5xx\": 502 } }\n",
    );
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("warning: endpoint 'GET /old': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)"), "{err}");
    assert!(err.contains("warning: endpoint 'GET /health': via 'POST /orders': that endpoint's error_map is ignored here (each endpoint maps its own errors)"), "{err}");
}

#[test]
fn validate_places_errors_in_the_endpoints_file() {
    let dir = project("eps-errors", "endpoints: kaanal.endpoints.yaml\n");
    let out = kaanal(&dir, &["validate"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("kaanal.yaml:9:12: "),
        "a missing file, at the endpoints: value: {}",
        stderr(&out)
    );
    let file = dir.join("kaanal.endpoints.yaml");
    for (text, want) in [
        (
            "\"GET /users/{id}\":\n  chain:\n    - call: pay\n",
            "kaanal.endpoints.yaml:3:13: endpoint 'GET /users/{id}': unknown node 'pay'",
        ),
        (
            "\"GET /users/{id}\":\n  chain:\n    - { cache: c, op: read }\n",
            "kaanal.endpoints.yaml:3:7: ",
        ),
    ] {
        std::fs::write(&file, text).unwrap();
        let out = kaanal(&dir, &["validate"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains(want), "{}", stderr(&out));
    }
}
