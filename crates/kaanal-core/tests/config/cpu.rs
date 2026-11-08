//! `cpu: { cores }` (step 8, RULES 101): whole and fractional cores, the range, and where the
//! field is allowed. The engine sees integer millicores.

use kaanal_core::config::CpuConfig;
use kaanal_core::{Config, Topology};

fn load(app_fields: &str) -> Result<Config, String> {
    Config::from_yaml(&format!(
        "version: 1\nnodes:\n  app: {{ type: service{app_fields} }}\n"
    ))
    .map_err(|e| e.to_string())
}

fn millicores(app_fields: &str) -> Option<u32> {
    let t = Topology::build(&load(app_fields).unwrap()).unwrap();
    t.nodes[t.id("app").unwrap()].cpu
}

#[test]
fn whole_and_fractional_cores_become_millicores() {
    assert_eq!(millicores(", cpu: { cores: 4 }"), Some(4000));
    assert_eq!(millicores(", cpu: { cores: 0.5 }"), Some(500));
    assert_eq!(millicores(", cpu: { cores: 0.001 }"), Some(1));
    assert_eq!(millicores(", cpu: { cores: 4096 }"), Some(4_096_000));
    assert_eq!(millicores(", cpu: { cores: 2.25 }"), Some(2250));
}

#[test]
fn absent_means_no_sharing() {
    assert_eq!(millicores(""), None);
}

#[test]
fn a_bad_number_is_refused_at_its_value_with_the_node_named() {
    for bad in ["0", "-1", "5000", "4097", "0.0001", ".nan"] {
        let err = load(&format!(", cpu: {{ cores: {bad} }}")).unwrap_err();
        assert!(
            err.contains("node 'app': cpu cores must be a number from 0.001 to 4096"),
            "{bad}: {err}"
        );
        assert!(
            err.starts_with("3:"),
            "{bad} is positioned on its line: {err}"
        );
    }
}

#[test]
fn text_is_not_a_number() {
    let err = load(", cpu: { cores: x }").unwrap_err();
    assert!(err.contains("invalid type"), "{err}");
}

#[test]
fn an_unknown_key_and_a_missing_cores_are_refused() {
    assert!(load(", cpu: { core: 2 }").is_err());
    assert!(load(", cpu: {}").is_err());
}

#[test]
fn kinds_without_compute_of_their_own_do_not_take_the_field() {
    for node in [
        "{ type: cache, cpu: { cores: 2 } }",
        "{ type: external-api, cpu: { cores: 2 } }",
        "{ type: storage, cpu: { cores: 2 } }",
    ] {
        let yaml = format!("version: 1\nnodes:\n  app: {{ type: service }}\n  x: {node}\n");
        assert!(Config::from_yaml(&yaml).is_err(), "{node}");
    }
}

#[test]
fn gateway_database_auth_and_monolith_take_it() {
    for node in [
        "{ type: gateway, cpu: { cores: 2 } }",
        "{ type: database, flavor: postgres, cpu: { cores: 2 } }",
        "{ type: monolith, cpu: { cores: 2 } }",
    ] {
        let yaml = format!("version: 1\nnodes:\n  x: {node}\n");
        let c = Config::from_yaml(&yaml).unwrap_or_else(|e| panic!("{node}: {e}"));
        assert_eq!(
            c.nodes["x"].cpu().map(CpuConfig::millicores),
            Some(2000),
            "{node}"
        );
    }
}
