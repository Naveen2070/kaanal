//! Locale tags on nodes and the project.

use crate::common::*;
use kaanal_core::{Config, Topology};

#[test]
fn node_locale_and_project_locale_parse_and_default() {
    let cfg = Config::from_yaml(
        "version: 1\nlocale: en-US\ndata: more/data.yaml\nnodes:\n  app: { type: service, locale: ja-JP }\n  db: { type: database, flavor: postgres }\n  gw: { type: gateway, locale: de-DE }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n",
    )
    .unwrap();
    assert_eq!(cfg.locale.as_deref(), Some("en-US"));
    assert_eq!(cfg.data.as_deref(), Some("more/data.yaml"));
    assert_eq!(cfg.nodes["app"].locale(), Some("ja-JP"));
    assert_eq!(cfg.nodes["db"].locale(), None);
    assert_eq!(cfg.nodes["gw"].locale(), Some("de-DE"));
    let topo = Topology::build(&cfg).unwrap();
    let app = topo.nodes.iter().find(|n| n.name == "app").unwrap();
    assert_eq!(app.locale.as_deref(), Some("ja-JP"));
    // Both optional: absent means the default (en-IN, applied by the server).
    let bare = Config::from_yaml(SAMPLE).unwrap();
    assert_eq!(bare.locale, None);
    assert_eq!(bare.data, None);
}

#[test]
fn locale_tags_must_match_the_pattern_before_any_path_use() {
    for ok in [
        "en",
        "en-IN",
        "ja-JP",
        "zh-Hant-TW",
        "sr-Latn-RS",
        "EN-in",
        "gsw-CH",
    ] {
        assert!(kaanal_core::valid_locale_tag(ok), "{ok}");
    }
    for bad in [
        "",
        "../../x",
        "a/b",
        "en_IN",
        "en-",
        "-en",
        "e",
        "engl-IN",
        "en-I",
        "en-IN-toolongtag",
        "en IN",
        r"en\IN",
        "..",
        "en-IN.yaml",
        "ü-IN",
    ] {
        assert!(!kaanal_core::valid_locale_tag(bad), "{bad}");
    }
}

#[test]
fn a_bad_locale_tag_is_an_error_at_its_value() {
    let e =
        Config::from_yaml("version: 1\nlocale: ../../etc/x\nnodes:\n  app: { type: service }\n")
            .unwrap_err();
    assert_eq!((e.line, e.col), (Some(2), Some(9)), "{e}");
    assert!(
        e.msg.contains("../../etc/x") && e.msg.contains("en-IN"),
        "{e}"
    );
    let e = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, locale: en_IN }\n",
    )
    .unwrap_err();
    assert_eq!((e.line, e.col), (Some(4), Some(33)), "{e}");
    assert!(e.msg.contains("node 'app'"), "{e}");
}
