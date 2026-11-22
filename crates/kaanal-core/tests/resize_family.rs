//! The database-family candidates (RULES 141, Step 9c): the rule table is the oracle (relational
//! against non-relational and back; no managed class for Cassandra or `DynamoDB`; written ops kept).

use kaanal_core::Config;
use kaanal_core::config::{DbFlavor, Hosting};
use kaanal_core::resize::{Family, alternatives, family_of};

fn cfg(nodes: &str) -> Config {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
}

fn flavors(c: &Config, node: &str, graph: bool) -> Vec<DbFlavor> {
    alternatives(c, node, &[], graph)
        .unwrap()
        .kept
        .iter()
        .map(|a| a.flavor)
        .collect()
}

#[test]
fn relational_is_tried_against_non_relational_and_back() {
    let c = cfg("  d: { type: database, flavor: postgres }\n");
    assert_eq!(
        flavors(&c, "d", false),
        vec![DbFlavor::Mongodb, DbFlavor::Dynamodb, DbFlavor::Cassandra]
    );
    let c = cfg("  d: { type: database, flavor: mongodb }\n");
    assert_eq!(
        flavors(&c, "d", false),
        vec![
            DbFlavor::Postgres,
            DbFlavor::Mysql,
            DbFlavor::Mariadb,
            DbFlavor::Sqlserver,
            DbFlavor::Oracle
        ]
    );
}

#[test]
fn neo4j_only_on_request() {
    let c = cfg("  d: { type: database, flavor: mysql }\n");
    assert!(!flavors(&c, "d", false).contains(&DbFlavor::Neo4j));
    assert!(flavors(&c, "d", true).contains(&DbFlavor::Neo4j));
}

#[test]
fn a_database_with_written_ops_gives_a_reason_and_no_alternative() {
    let c = cfg(
        "  d: { type: database, flavor: postgres, ops: { read: 2ms, write: 4ms, scan: 9ms } }\n  s: { type: service, proc: 5ms }\n",
    );
    let why = alternatives(&c, "d", &[], false).unwrap_err();
    assert!(why.contains("your own ops"), "{why}");
    assert_eq!(
        alternatives(&c, "s", &[], false).unwrap_err(),
        "not a database"
    );
    assert_eq!(
        alternatives(&c, "nope", &[], false).unwrap_err(),
        "not in the design"
    );
    let c = cfg("  d: { type: database, flavor: postgres }\n");
    assert_eq!(
        alternatives(&c, "d", &["d".into()], false).unwrap_err(),
        "kept by you"
    );
}

#[test]
fn a_managed_postgres_becomes_a_self_hosted_dynamodb_with_a_note() {
    let c = cfg(
        "  d: { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.large, multi_az: true }\n",
    );
    let alts = alternatives(&c, "d", &[], false).unwrap();
    let dynamo = alts
        .kept
        .iter()
        .find(|a| a.flavor == DbFlavor::Dynamodb)
        .expect("dynamodb kept");
    let d = &dynamo.config.nodes["d"];
    assert_eq!(d.hosting_written(), Some(Hosting::SelfHosted));
    assert_eq!(d.instance_name(), Some("aws/m5.large"));
    assert_eq!(d.multi_az_written(), None);
    assert_eq!(d.engine_written(), None);
    assert!(
        dynamo.notes.iter().any(|n| n.contains("no managed class")),
        "{:?}",
        dynamo.notes
    );
    // the design it came from is untouched
    assert_eq!(c.nodes["d"].hosting_written(), Some(Hosting::Managed));
}

#[test]
fn a_managed_node_with_your_own_instance_keeps_it_when_it_becomes_self_hosted() {
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  big: { cores: 8, cost_per_hour: 1.5 }\nnodes:\n  d: { type: database, flavor: postgres, hosting: managed, instance: big }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let alts = alternatives(&c, "d", &[], false).unwrap();
    let a = alts
        .kept
        .iter()
        .find(|a| a.flavor == DbFlavor::Cassandra)
        .expect("cassandra kept");
    assert_eq!(a.config.nodes["d"].instance_name(), Some("big"));
    assert!(
        a.notes
            .iter()
            .any(|n| n.contains("your instance big is kept")),
        "{:?}",
        a.notes
    );
}

#[test]
fn an_alternative_that_fails_the_hosting_checks_is_dropped_with_a_note() {
    // an Azure SQL class does not run MongoDB: dropped with the reason, the rest stay
    let c = cfg(
        "  d: { type: database, flavor: sqlserver, hosting: managed, instance: azure/db.gp_gen5_4 }\n",
    );
    let alts = alternatives(&c, "d", &[], false).unwrap();
    let why = alts
        .dropped
        .iter()
        .find(|(f, _)| *f == DbFlavor::Mongodb)
        .map(|(_, w)| w.as_str())
        .expect("mongodb dropped");
    assert!(why.contains("Azure SQL"), "{why}");
}

#[test]
fn family_of_covers_every_flavor() {
    use DbFlavor::{
        Cassandra, Dynamodb, Mariadb, Mongodb, Mysql, Neo4j, Oracle, Postgres, Sqlserver,
    };
    // a match with no wildcard: a tenth flavor stops this compiling
    for f in [
        Postgres, Mysql, Mariadb, Sqlserver, Oracle, Mongodb, Dynamodb, Cassandra, Neo4j,
    ] {
        let rel = match f {
            Postgres | Mysql | Mariadb | Sqlserver | Oracle => true,
            Mongodb | Dynamodb | Cassandra | Neo4j => false,
        };
        assert_eq!(family_of(f) == Family::Relational, rel, "{f:?}");
    }
}
