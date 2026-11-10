//! `instance:` (step 8P) and `memory:` (step 8M): what an instance fills, that a written value wins,
//! the refusals with their path, and how the memory becomes a slot count.

use kaanal_core::{Config, Topology};

fn load(app_fields: &str) -> Result<Config, String> {
    Config::from_yaml(&format!(
        "version: 1\nnodes:\n  app: {{ type: service{app_fields} }}\n"
    ))
    .map_err(|e| e.to_string())
}

fn node(app_fields: &str) -> kaanal_core::Node {
    let t = Topology::build(&load(app_fields).unwrap()).unwrap();
    t.nodes[t.id("app").unwrap()].clone()
}

#[test]
fn an_instance_fills_the_cores_and_both_directions_of_the_card() {
    let n = node(", instance: aws/m5.large");
    assert_eq!(n.cpu, Some(2000));
    assert_eq!(n.bandwidth, (Some(1_250_000_000), Some(1_250_000_000)));
    assert!(n.memory.is_none(), "an instance alone sets no memory model");
}

#[test]
fn what_the_node_writes_wins_over_the_instance() {
    let n = node(", instance: aws/m5.large, cpu: { cores: 1.5 }, bandwidth: { out: 1MB }");
    assert_eq!(n.cpu, Some(1500));
    // a written card is used as written: the side left out stays unlimited, not the instance's
    assert_eq!(n.bandwidth, (Some(1 << 20), None));
}

#[test]
fn the_name_is_case_insensitive_and_an_unknown_one_is_refused_at_its_value() {
    assert_eq!(node(", instance: AWS/M5.Large").cpu, Some(2000));
    let err = load(", instance: aws/m5.huge").unwrap_err();
    assert!(
        err.contains("node 'app': instance 'aws/m5.huge' is not in the table")
            && err.contains("aws/m5.large"),
        "{err}"
    );
    assert!(err.starts_with("3:"), "positioned on its line: {err}");
    let err = load(", instance: mars/x1").unwrap_err();
    assert!(err.contains("provider/type"), "{err}");
}

#[test]
fn memory_is_a_slot_count_and_the_limit_may_come_from_the_instance() {
    // (8 GiB - 1 GiB) / 512 MiB = 14
    let n = node(", instance: aws/m5.large, memory: { per_request: 512MiB, base: 1GiB }");
    let m = n.memory.unwrap();
    assert_eq!(m.slots(), 14);
    assert_eq!(n.admission.as_ref().map(|a| a.slots), Some(14));
    // no concurrency of its own: memory does not wait, it refuses at once
    assert_eq!(n.admission.as_ref().unwrap().queue, Some(0));
    assert_eq!(n.admission.as_ref().unwrap().overflow_status, 503);
}

#[test]
fn memory_lowers_concurrency_and_never_raises_it() {
    let n = node(
        ", concurrency: { limit: 100, queue: 50 }, memory: { limit: 1GB, per_request: 100MB }",
    );
    let a = n.admission.unwrap();
    assert_eq!(
        (a.slots, a.queue),
        (10, Some(50)),
        "10 fit in memory, the queue stays"
    );
    let n = node(", concurrency: 4, memory: { limit: 1GB, per_request: 100MB }");
    assert_eq!(
        n.admission.unwrap().slots,
        4,
        "concurrency was already lower"
    );
}

#[test]
fn no_memory_changes_nothing() {
    let n = node(", concurrency: 8");
    assert_eq!(n.admission.unwrap().slots, 8);
    assert!(node("").admission.is_none());
}

#[test]
fn a_memory_that_cannot_hold_one_request_or_lacks_a_field_is_refused() {
    for (fields, want) in [
        (", memory: { limit: 1GB }", "per_request is required"),
        (", memory: { per_request: 1MB }", "limit is required"),
        (
            ", memory: { limit: 1MB, per_request: 2MB }",
            "no room for one request",
        ),
        (
            ", memory: { limit: 2MB, per_request: 1MB, base: 1500KB }",
            "no room for one request",
        ),
        (
            ", memory: { limit: 0, per_request: 1MB }",
            "limit is required",
        ),
    ] {
        let err = load(fields).unwrap_err();
        assert!(
            err.contains("node 'app': memory:") && err.contains(want),
            "{fields}: {err}"
        );
        assert!(err.starts_with("3:"), "{fields}: {err}");
    }
}

#[test]
fn sizes_take_units_above_one_gib_and_refuse_junk() {
    let n = node(", memory: { limit: 64GB, per_request: 8MB }");
    assert_eq!(n.memory.unwrap().limit, 64 << 30);
    assert!(load(", memory: { limit: lots, per_request: 8MB }").is_err());
    assert!(load(", memory: { limit: 1GB, per_request: 1MB, speed: 3 }").is_err());
}

#[test]
fn only_the_kinds_with_a_cpu_take_instance_and_memory() {
    let cache = Config::from_yaml(
        "version: 1\nnodes:\n  c: { type: cache, flavor: redis, instance: aws/m5.large }\n",
    );
    assert!(cache.is_err(), "a cache has no instance field");
}
