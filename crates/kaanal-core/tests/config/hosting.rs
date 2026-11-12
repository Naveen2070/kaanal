//! `hosting:` and `engine:` (RULES 122): what each kind may be hosted as, which `instance:` goes
//! with which hosting, and that a managed database class fills cores, memory and the card like a VM.

use kaanal_core::Config;

fn load(nodes: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).map_err(|e| e.to_string())
}

fn db(fields: &str) -> Result<Config, String> {
    load(&format!(
        "  d: {{ type: database, ops: {{ read: 5ms, write: 8ms, scan: 20ms }}{fields} }}\n"
    ))
}

#[test]
fn a_node_with_no_hosting_is_self_hosted_and_takes_a_vm_type() {
    let c = db(", instance: aws/m5.large").unwrap();
    assert_eq!(
        c.nodes["d"].hosting(),
        kaanal_core::config::Hosting::SelfHosted
    );
}

#[test]
fn a_managed_database_takes_a_class_and_the_class_fills_cores_memory_and_the_card() {
    let c = db(", hosting: managed, engine: mysql, instance: aws/db.m5.large, memory: { per_request: 64MiB }").unwrap();
    let n = &c.nodes["d"];
    assert_eq!(n.cpu().unwrap().cores, 2.0);
    assert_eq!(n.bandwidth(), (Some(1_250_000_000), Some(1_250_000_000)));
    assert_eq!(c.nodes["d"].engine(), kaanal_core::config::DbEngine::Mysql);
    assert_eq!(c.nodes["d"].memory().unwrap().limit, 8 << 30);
}

#[test]
fn the_engine_defaults_to_postgresql() {
    let c = db(", hosting: managed, instance: aws/db.m5.large").unwrap();
    assert_eq!(
        c.nodes["d"].engine(),
        kaanal_core::config::DbEngine::Postgresql
    );
}

#[test]
fn a_vm_type_and_a_managed_class_do_not_mix() {
    let e = db(", instance: aws/db.m5.large").unwrap_err();
    assert!(
        e.contains("is a managed class") && e.contains("hosting: managed"),
        "{e}"
    );
    let e = db(", hosting: managed, instance: aws/m5.large").unwrap_err();
    assert!(
        e.contains("is not a database class") && e.contains("aws/db.m5.large"),
        "{e}"
    );
}

#[test]
fn serverless_and_external_have_no_instance() {
    for h in ["serverless", "external"] {
        let e = db(&format!(", hosting: {h}, instance: aws/m5.large")).unwrap_err();
        assert!(e.contains(&format!("hosting: {h} has no instance")), "{e}");
    }
    // without an instance they load, and cores and memory stay yours to write
    let c = db(", hosting: serverless, cpu: { cores: 2 }").unwrap();
    assert_eq!(
        c.nodes["d"].hosting(),
        kaanal_core::config::Hosting::Serverless
    );
}

#[test]
fn a_kind_takes_only_the_hostings_that_exist_for_it() {
    // a service is a VM or serverless, a managed service is a database or a gateway
    let e = load("  s: { type: service, hosting: managed }\n").unwrap_err();
    assert!(e.contains("a service takes self, serverless"), "{e}");
    let e = load("  g: { type: gateway, hosting: external }\n").unwrap_err();
    assert!(e.contains("a gateway takes self, managed"), "{e}");
    assert!(load("  s: { type: service, hosting: serverless }\n").is_ok());
    assert!(load("  g: { type: gateway, hosting: managed }\n").is_ok());
    assert!(load("  a: { type: auth, hosting: external }\n").is_ok());
}

#[test]
fn engine_goes_only_with_a_managed_database() {
    let e = db(", engine: mysql").unwrap_err();
    assert!(
        e.contains("engine goes with a database that has hosting: managed"),
        "{e}"
    );
}

#[test]
fn a_managed_gateway_has_no_instance() {
    let e = load("  g: { type: gateway, hosting: managed, instance: aws/m5.large }\n").unwrap_err();
    assert!(e.contains("hosting: managed has no instance"), "{e}");
}

#[test]
fn your_own_instance_goes_with_self_or_a_managed_database() {
    let y = |h: &str| {
        Config::from_yaml(&format!(
            "version: 1\ninstances:\n  rack: {{ cores: 8 }}\nnodes:\n  d: {{ type: database, ops: {{ read: 5ms, write: 8ms, scan: 20ms }}, instance: rack{h} }}\n"
        ))
        .map_err(|e| e.to_string())
    };
    assert!(y("").is_ok());
    assert!(y(", hosting: managed").is_ok());
    assert!(y(", hosting: serverless").is_err());
}

fn cache(fields: &str) -> Result<Config, String> {
    load(&format!("  c: {{ type: cache, flavor: redis{fields} }}\n"))
}

#[test]
fn a_cache_is_a_vm_you_run_a_managed_service_or_serverless() {
    use kaanal_core::config::Hosting;
    assert!(cache("").is_ok());
    assert!(cache(", instance: aws/m5.large").is_ok());
    let c = cache(", hosting: managed, instance: aws/cache.m5.large").unwrap();
    assert_eq!(c.nodes["c"].hosting(), Hosting::Managed);
    // the class names a size for the price and the note, but fills no cores: a cache's hits are fixed times
    assert!(c.nodes["c"].instance().is_some() && c.nodes["c"].cpu().is_none());
    assert!(cache(", hosting: serverless").is_ok());
    assert!(cache(", hosting: external").is_err());
}

#[test]
fn a_cache_class_and_a_database_class_do_not_cross() {
    let e = cache(", hosting: managed, instance: aws/db.m5.large").unwrap_err();
    assert!(
        e.contains("is not a cache class") && e.contains("aws/cache.m5.large"),
        "{e}"
    );
    let e = cache(", instance: aws/cache.m5.large").unwrap_err();
    assert!(e.contains("is a managed class"), "{e}");
    let e = db(", hosting: managed, instance: aws/cache.m5.large").unwrap_err();
    assert!(e.contains("is not a database class"), "{e}");
    let e = cache(", hosting: serverless, instance: aws/m5.large").unwrap_err();
    assert!(e.contains("hosting: serverless has no instance"), "{e}");
}

#[test]
fn a_managed_class_runs_only_the_flavors_the_table_has_and_the_engine_follows_the_flavor() {
    use kaanal_core::config::DbEngine;
    let flavored = |flavor: &str, fields: &str| {
        load(&format!(
            "  d: {{ type: database, flavor: {flavor}, hosting: managed{fields} }}\n"
        ))
    };
    // the engine follows the flavor when it is not written
    let c = flavored("mysql", ", instance: aws/db.m5.large").unwrap();
    assert_eq!(c.nodes["d"].engine(), DbEngine::Mysql);
    let c = flavored("mariadb", ", instance: aws/db.m5.large").unwrap();
    assert_eq!(c.nodes["d"].engine(), DbEngine::Mariadb);
    let c = flavored(
        "postgres",
        ", engine: aurora-postgresql, instance: aws/db.m5.large",
    )
    .unwrap();
    assert_eq!(c.nodes["d"].engine(), DbEngine::AuroraPostgresql);
    // an engine that does not run the flavor is refused
    let e = flavored("postgres", ", engine: mysql").unwrap_err();
    assert!(
        e.contains("does not run postgres") && e.contains("postgresql or aurora-postgresql"),
        "{e}"
    );
    // MongoDB, Neo4j, Cassandra and DynamoDB have no class in the table
    for f in ["mongodb", "neo4j", "cassandra", "dynamodb"] {
        let e = flavored(f, ", instance: aws/db.m5.large").unwrap_err();
        assert!(
            e.contains(&format!("{f} has no managed class in the table")),
            "{e}"
        );
        let e = flavored(f, ", engine: mysql").unwrap_err();
        assert!(e.contains("has no managed engine in the table"), "{e}");
        // managed with no instance loads: size and price are yours
        assert!(flavored(f, ", cpu: { cores: 4 }").is_ok(), "{f}");
    }
}

#[test]
fn the_new_database_flavors_fill_the_op_costs() {
    for f in ["mariadb", "neo4j", "cassandra", "dynamodb"] {
        let c = db_with_flavor(f);
        assert!(c.is_ok(), "{f}: {c:?}");
    }
}

fn db_with_flavor(f: &str) -> Result<Config, String> {
    load(&format!("  d: {{ type: database, flavor: {f} }}\n"))
}
