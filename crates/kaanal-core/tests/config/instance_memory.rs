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
            && err.contains("aws has "),
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

#[test]
fn on_limit_kill_sets_a_restart_that_defaults_to_ten_seconds() {
    let secs = |f: &str| node(f).oom_restart.map(|d| d.as_secs_f64());
    assert_eq!(
        secs(", memory: { limit: 1GB, per_request: 100MB, on_limit: kill }"),
        Some(10.0)
    );
    assert_eq!(
        secs(", memory: { limit: 1GB, per_request: 100MB, on_limit: kill, restart: 500ms }"),
        Some(0.5)
    );
    assert_eq!(
        secs(", memory: { limit: 1GB, per_request: 100MB }"),
        None,
        "refuse is the default"
    );
    assert_eq!(
        secs(", memory: { limit: 1GB, per_request: 100MB, on_limit: refuse }"),
        None
    );
}

#[test]
fn a_node_dies_of_memory_only_when_memory_is_what_limits_its_slots() {
    // concurrency 4 is lower than the 10 requests the memory holds: a refusal is the slot limit's
    let n = node(", concurrency: 4, memory: { limit: 1GB, per_request: 100MB, on_limit: kill }");
    assert_eq!(n.oom_restart, None);
    assert_eq!(n.admission.unwrap().slots, 4);
    let n = node(", concurrency: 50, memory: { limit: 1GB, per_request: 100MB, on_limit: kill }");
    assert!(n.oom_restart.is_some(), "memory holds 10, below 50");
}

#[test]
fn restart_needs_kill_and_must_be_a_sensible_duration() {
    for (fields, want) in [
        (
            ", memory: { limit: 1GB, per_request: 1MB, restart: 5s }",
            "restart needs on_limit: kill",
        ),
        (
            ", memory: { limit: 1GB, per_request: 1MB, on_limit: kill, restart: 0s }",
            "above 0",
        ),
        (
            ", memory: { limit: 1GB, per_request: 1MB, on_limit: kill, restart: 2h }",
            "at most 1h",
        ),
        (
            ", memory: { limit: 1GB, per_request: 1MB, on_limit: kill, restart: soon }",
            "use a duration",
        ),
    ] {
        let err = load(fields).unwrap_err();
        assert!(err.contains(want), "{fields}: {err}");
    }
    assert!(load(", memory: { limit: 1GB, per_request: 1MB, on_limit: explode }").is_err());
}

// ---- your own instances (`instances:` at the top of the file) ----

fn load_with(instances: &str, app_fields: &str) -> Result<Config, String> {
    Config::from_yaml(&format!(
        "version: 1\ninstances:\n{instances}nodes:\n  app: {{ type: service{app_fields} }}\n"
    ))
    .map_err(|e| e.to_string())
}

fn node_with(instances: &str, app_fields: &str) -> kaanal_core::Node {
    let t = Topology::build(&load_with(instances, app_fields).unwrap()).unwrap();
    t.nodes[t.id("app").unwrap()].clone()
}

const BOX: &str = "  build-box: { cores: 32, memory: 128GB, bandwidth: 10GB }\n";

#[test]
fn a_custom_instance_fills_cores_card_and_memory_like_a_table_type() {
    let n = node_with(BOX, ", instance: build-box, memory: { per_request: 1GB }");
    assert_eq!(n.cpu, Some(32_000));
    assert_eq!(n.bandwidth, (Some(10 << 30), Some(10 << 30)));
    assert_eq!(n.memory.unwrap().limit, 128 << 30);
    assert_eq!(n.admission.unwrap().slots, 128);
}

#[test]
fn what_the_node_writes_wins_over_a_custom_instance_and_a_partial_one_fills_only_what_it_has() {
    let n = node_with(
        BOX,
        ", instance: build-box, cpu: { cores: 4 }, bandwidth: 1MB",
    );
    assert_eq!(n.cpu, Some(4000));
    assert_eq!(n.bandwidth, (Some(1 << 20), Some(1 << 20)));
    // only cores: no card, no memory model
    let n = node_with("  tiny: { cores: 0.5 }\n", ", instance: tiny");
    assert_eq!((n.cpu, n.bandwidth), (Some(500), (None, None)));
    assert!(n.memory.is_none());
}

#[test]
fn custom_instances_are_shared_by_name_and_a_table_type_still_works_beside_them() {
    let cfg = "version: 1\ninstances:\n  rack: { cores: 8 }\nnodes:\n  a: { type: service, instance: rack }\n  b: { type: service, instance: rack }\n  c: { type: service, instance: aws/m5.large }\n";
    let t = Topology::build(&Config::from_yaml(cfg).unwrap()).unwrap();
    let cores = |n: &str| t.nodes[t.id(n).unwrap()].cpu;
    assert_eq!(
        (cores("a"), cores("b"), cores("c")),
        (Some(8000), Some(8000), Some(2000))
    );
}

#[test]
fn a_bad_custom_instance_or_an_unknown_name_is_refused_where_it_is_written() {
    for (instances, fields, want) in [
        (
            "  x: { cores: 0 }\n",
            "",
            "cores must be a number from 0.001 to 4096",
        ),
        ("  x: { cores: 5000 }\n", "", "cores must be a number"),
        ("  x: { memory: 1GB }\n", "", "cores"),
        (
            "  x: { cores: 2, memory: 0 }\n",
            "",
            "memory must be at least 1 byte",
        ),
        (
            "  x: { cores: 2, bandwidth: 0 }\n",
            "",
            "bandwidth must be at least 1",
        ),
        ("  a/b: { cores: 2 }\n", "", "not a usable name"),
        ("  '': { cores: 2 }\n", "", "not a usable name"),
        ("  x: { cores: 2, speed: 3 }\n", "", "speed"),
        ("  x: { cores: 2 }\n", ", instance: y", "your instances: x"),
        (
            "  x: { cores: 2 }\n",
            ", instance: aws/m5.huge",
            "not in the table or in your instances",
        ),
    ] {
        let err = load_with(instances, fields).unwrap_err();
        assert!(err.contains(want), "{instances}{fields}: {err}");
    }
}
