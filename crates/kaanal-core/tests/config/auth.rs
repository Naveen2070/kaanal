//! Auth claims, scopes, users, passwords and token ttl.

use kaanal_core::Config;
use kaanal_core::config::NodeConfig;
use serde_json::json;
use std::fmt::Write as _;
use std::time::Duration;

#[test]
fn auth_claims_and_scopes_parse() {
    let cfg = Config::from_yaml(
        "version: 1
nodes:
  login:
    type: auth
    token_ttl: 2h
    claims: { aud: my-app, level: 7, beta: true }
    users:
      - { username: alice, password: secret, roles: [admin], scopes: [read, write], claims: { email: a@x.dev } }
      - { username: bob, password: pw }
",
    )
    .unwrap();
    let NodeConfig::Auth(a) = &cfg.nodes["login"] else {
        panic!("auth")
    };
    assert_eq!(a.claims["aud"], json!("my-app"));
    assert_eq!(a.claims["level"], json!(7));
    assert_eq!(a.claims["beta"], json!(true));
    assert_eq!(a.users[0].scopes, ["read", "write"]);
    assert_eq!(a.users[0].claims["email"], json!("a@x.dev"));
    assert!(a.users[1].scopes.is_empty() && a.users[1].claims.is_empty());
    assert_eq!(a.token_ttl, Some(Duration::from_secs(7200)));
}

#[test]
fn reserved_claim_on_a_later_user_points_at_that_claim() {
    // Spec §7: an error on user #57 points at that key, not at `users:` (serde alone would).
    let mut yaml = String::from("version: 1\nnodes:\n  login:\n    type: auth\n    users:\n");
    for i in 0..58 {
        let extra = if i == 57 { ", claims: { iss: x }" } else { "" };
        let _ = writeln!(yaml, "      - {{ username: u{i}, password: p{extra} }}");
    }
    let err = Config::from_yaml(&yaml).unwrap_err();
    assert_eq!((err.line, err.col), (Some(63), Some(49)), "{err}");
    assert!(
        err.msg
            .starts_with("node 'login': users[57].claims: 'iss' is set by Kaanal"),
        "{err}"
    );
}

#[test]
fn auth_rules_are_checked_after_parsing_and_placed() {
    let long = "x".repeat(257);
    let many_roles = (0..33)
        .map(|i| format!("r{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let many_claims = (0..33)
        .map(|i| format!("c{i}: 1"))
        .collect::<Vec<_>>()
        .join(", ");
    let cases: Vec<(String, (usize, usize), &str)> = vec![
        (
            "    claims: { sub: x }\n".into(),
            (5, 15),
            "claims: 'sub' is set by Kaanal",
        ),
        (
            "    claims: { a: [1] }\n".into(),
            (5, 18),
            "claims.a: must be a string",
        ),
        (
            "    claims: { a: null }\n".into(),
            (5, 18),
            "claims.a: must be a string",
        ),
        (
            format!("    claims: {{ a: {long} }}\n"),
            (5, 18),
            "claims.a: must be a string (at most 256 bytes)",
        ),
        (
            format!("    claims: {{ {many_claims} }}\n"),
            (5, 5),
            "claims: at most 32 claims (got 33)",
        ),
        (
            "    users:\n      - { username: \"\", password: p }\n".into(),
            (6, 21),
            "users[0].username: must be 1 to 256 bytes",
        ),
        (
            format!("    users:\n      - {{ username: a, password: {long} }}\n"),
            (6, 34),
            "users[0].password: must be at most 256 bytes",
        ),
        (
            format!("    users:\n      - {{ username: a, password: p, roles: [{many_roles}] }}\n"),
            (6, 37),
            "users[0].roles: at most 32 entries (got 33)",
        ),
        (
            "    users:\n      - { username: a, password: p, scopes: [\"bad scope\"] }\n".into(),
            (6, 46),
            "users[0].scopes[0]: must be 1 to 256 printable ASCII characters",
        ),
        (
            "    users:\n      - { username: a, password: p, scopes: [\"\"] }\n".into(),
            (6, 46),
            "users[0].scopes[0]: must be 1 to 256 printable ASCII characters",
        ),
    ];
    for (body, (line, col), want) in cases {
        let yaml = format!("version: 1\nnodes:\n  login:\n    type: auth\n{body}");
        let err = Config::from_yaml(&yaml).unwrap_err();
        assert_eq!(
            (err.line, err.col),
            (Some(line), Some(col)),
            "{body}: {err}"
        );
        assert!(err.msg.contains(want), "{body}: want '{want}' in: {err}");
        // A rejected value (a password above all) is never echoed.
        assert!(!err.msg.contains("xxxx"), "{body}: value echoed: {err}");
    }
    // 1000 users load; 1001 are rejected at `users:`.
    let users = |n: usize| {
        let mut s = String::new();
        for i in 0..n {
            let _ = writeln!(s, "      - {{ username: u{i}, password: p }}");
        }
        s
    };
    let head = "version: 1\nnodes:\n  login:\n    type: auth\n    users:\n";
    Config::from_yaml(&format!("{head}{}", users(1000))).unwrap();
    let err = Config::from_yaml(&format!("{head}{}", users(1001))).unwrap_err();
    assert_eq!((err.line, err.col), (Some(5), Some(5)), "{err}");
    assert!(err.msg.contains("at most 1000 users (got 1001)"), "{err}");
}

#[test]
fn a_non_text_password_is_rejected_at_its_value_without_echo() {
    // serde alone would say `invalid type: integer `4711`` at the node's key.
    let cases = [
        ("4711", "a number"),
        ("1.5", "a number"),
        ("true", "a boolean"),
        ("null", "null"),
    ];
    for (pw, kind) in cases {
        for body in [
            format!("      - {{ username: a, password: {pw} }}\n"),
            format!("      - username: a\n        password: {pw}\n"),
        ] {
            let yaml = format!("version: 1\nnodes:\n  login:\n    type: auth\n    users:\n{body}");
            let err = Config::from_yaml(&yaml).unwrap_err();
            let (line, col) = if body.starts_with("      - {") {
                (6, 34)
            } else {
                (7, 19)
            };
            assert_eq!((err.line, err.col), (Some(line), Some(col)), "{pw}: {err}");
            assert!(
                err.msg.contains(&format!(
                    "users[0].password: must be text, not {kind} (put it in quotes)"
                )),
                "{pw}: {err}"
            );
            // (`null` names its own kind in the message; the others must not appear at all.)
            assert!(
                pw == "null" || !err.msg.contains(pw),
                "{pw}: value echoed: {err}"
            );
        }
    }
    // A quoted number is text and loads; so does a number with a leading zero (this YAML
    // reader keeps `0123` and `007` as text), zeros intact.
    for (written, want) in [("\"4711\"", "4711"), ("0123", "0123"), ("007", "007")] {
        let cfg = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login: {{ type: auth, users: [ {{ username: a, password: {written} }} ] }}\n"
        ))
        .unwrap();
        let Some(NodeConfig::Auth(a)) = cfg.nodes.get("login") else {
            panic!("login is an auth node")
        };
        assert_eq!(a.users[0].password, want, "{written}");
    }
}

#[test]
fn token_ttl_must_be_whole_seconds_of_at_least_one() {
    // P1: placed at the value (5:16) by the per-field probe, like the 1-day cap.
    for bad in ["1500ms", "0s", "500ms"] {
        let err = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login:\n    type: auth\n    token_ttl: {bad}\n"
        ))
        .unwrap_err();
        assert_eq!((err.line, err.col), (Some(5), Some(16)), "{bad}: {err}");
        assert!(
            err.msg
                .contains("token_ttl must be whole seconds, at least 1s"),
            "{bad}: {err}"
        );
    }
    for ok in ["1s", "90s", "24h"] {
        Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login: {{ type: auth, token_ttl: {ok} }}\n"
        ))
        .unwrap();
    }
}

#[test]
fn user_debug_hides_the_password() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  login: { type: auth, users: [ { username: alice, password: hunter2-pw } ] }\n",
    )
    .unwrap();
    let text = format!("{cfg:?}");
    assert!(
        text.contains("alice") && text.contains("<redacted>"),
        "{text}"
    );
    assert!(!text.contains("hunter2"), "{text}");
}
