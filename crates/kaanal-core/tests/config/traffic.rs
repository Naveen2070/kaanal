//! `traffic:`: named load profiles for the simulator (weighted journeys over endpoints).

use kaanal_core::{Config, config_schema};
use std::time::Duration;

fn load(traffic: &str) -> Result<Config, String> {
    Config::from_yaml(&format!(
        "version: 1\nnodes:\n  app: {{ type: service }}\ntraffic:\n{traffic}"
    ))
    .map_err(|e| e.to_string())
}

const OK: &str = "  peak:\n    rps: 200\n    journeys:\n      - { endpoint: 'GET /products', weight: 70 }\n      - { endpoint: 'POST /orders', weight: 30, req_bytes: 2048, resp_bytes: 512 }\n";

#[test]
fn a_valid_profile_loads() {
    let c = load(&format!("{OK}    duration: 2m\n    warmup: 10s\n")).unwrap();
    let p = &c.traffic["peak"];
    assert!((p.rps - 200.0).abs() < f64::EPSILON);
    assert_eq!(p.journeys.len(), 2);
    assert_eq!(p.journeys[1].req_bytes, Some(2048));
    assert_eq!(p.journeys[0].resp_bytes, None);
    assert_eq!(p.duration, Some(Duration::from_secs(120)));
    assert_eq!(p.warmup, Some(Duration::from_secs(10)));
}

#[test]
fn a_journey_weight_defaults_to_one() {
    let c = load("  p:\n    rps: 1\n    journeys: [ { endpoint: 'GET /a' } ]\n").unwrap();
    assert!((c.traffic["p"].journeys[0].weight - 1.0).abs() < f64::EPSILON);
}

#[test]
fn weight_zero_nan_or_negative_is_rejected() {
    for bad in ["0", "-1", ".nan", ".inf"] {
        let err = load(&format!(
            "  p:\n    rps: 1\n    journeys:\n      - {{ endpoint: 'GET /a', weight: {bad} }}\n"
        ))
        .unwrap_err();
        assert!(err.contains("weight must be above 0"), "{bad}: {err}");
        assert!(err.starts_with("8:"), "{bad} is positioned: {err}");
    }
}

#[test]
fn rps_outside_the_range_is_rejected() {
    for bad in ["0", "-5", "0.0000001", "2000000", ".nan"] {
        let err = load(&format!(
            "  p:\n    rps: {bad}\n    journeys: [ {{ endpoint: 'GET /a' }} ]\n"
        ))
        .unwrap_err();
        assert!(err.contains("rps must be between"), "{bad}: {err}");
        assert!(err.starts_with("6:"), "{bad} is positioned: {err}");
    }
}

#[test]
fn a_duplicate_endpoint_in_one_profile_is_rejected() {
    let err = load("  p:\n    rps: 1\n    journeys:\n      - { endpoint: 'GET /a' }\n      - { endpoint: 'GET /a', weight: 2 }\n").unwrap_err();
    assert!(err.contains("'GET /a' appears twice"), "{err}");
    assert!(err.starts_with("9:"), "{err}");
}

#[test]
fn an_empty_or_huge_journey_list_is_rejected() {
    let err = load("  p:\n    rps: 1\n    journeys: []\n").unwrap_err();
    assert!(err.contains("at least one journey"), "{err}");
    let many = "      - { endpoint: 'GET /a' }
"
    .repeat(65);
    let err = load(&format!("  p:\n    rps: 1\n    journeys:\n{many}")).unwrap_err();
    assert!(err.contains("at most 64"), "{err}");
}

#[test]
fn bytes_are_bounded_and_names_are_checked() {
    let err = load(
        "  p:\n    rps: 1\n    journeys: [ { endpoint: 'GET /a', resp_bytes: 2000000000 } ]\n",
    )
    .unwrap_err();
    assert!(err.contains("resp_bytes must be at most"), "{err}");
    let err =
        load("  Bad Name:\n    rps: 1\n    journeys: [ { endpoint: 'GET /a' } ]\n").unwrap_err();
    assert!(err.contains("invalid") && err.contains("name"), "{err}");
}

#[test]
fn unknown_fields_are_rejected() {
    let err = load("  p:\n    rps: 1\n    users: 5\n    journeys: [ { endpoint: 'GET /a' } ]\n")
        .unwrap_err();
    assert!(err.contains("users"), "{err}");
}

#[test]
fn the_schema_lists_traffic() {
    let schema = config_schema().to_string();
    assert!(schema.contains("\"traffic\""));
    assert!(schema.contains("resp_bytes"));
}

#[test]
fn endpoints_are_compared_normalized_and_must_be_keys() {
    let err = load("  p:\n    rps: 1\n    journeys:\n      - { endpoint: 'GET /a' }\n      - { endpoint: 'get /a' }\n").unwrap_err();
    assert!(err.contains("appears twice"), "{err}");
    let err = load("  p:\n    rps: 1\n    journeys: [ { endpoint: 'nonsense' } ]\n").unwrap_err();
    assert!(err.contains("expected 'METHOD /path'"), "{err}");
}

#[test]
fn duration_and_warmup_are_checked_at_load() {
    for (extra, want) in [
        ("duration: 2h", "at most 1h"),
        ("duration: 0s", "above 0"),
        ("duration: 10s\n    warmup: 10s", "warmup must be shorter"),
    ] {
        let err = load(&format!(
            "  p:\n    rps: 1\n    {extra}\n    journeys: [ {{ endpoint: 'GET /a' }} ]\n"
        ))
        .unwrap_err();
        assert!(err.contains(want), "{extra}: {err}");
    }
    load("  p:\n    rps: 1\n    duration: 1h\n    warmup: 59m\n    journeys: [ { endpoint: 'GET /a' } ]\n").unwrap();
}
