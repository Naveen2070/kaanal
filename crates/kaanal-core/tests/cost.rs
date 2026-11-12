//! The cost estimate (RULES 121 to 126), held to AWS's published on-demand prices: the oracle is the
//! price list, not a number the code printed.

use std::collections::BTreeMap;

use kaanal_core::Config;
use kaanal_core::cost::{MULTI_AZ_FACTOR, estimate};

fn cfg(nodes: &str) -> Config {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
}

fn cost_of(c: &kaanal_core::cost::CostReport, name: &str) -> kaanal_core::cost::NodeCost {
    c.nodes.iter().find(|n| n.name == name).unwrap().clone()
}

/// EC2 t3.micro is $0.0104 an hour and RDS db.t3.micro PostgreSQL $0.018 in us-east-1 (AWS price
/// lists); copies multiply a node, Multi-AZ doubles a managed database (RDS bills a standby as a second
/// instance), and the total is the sum of what has a price.
#[test]
fn a_system_costs_copies_times_the_list_price_and_multi_az_doubles_a_database() {
    let c = estimate(
        &cfg("  app: { type: service, proc: 5ms, instance: aws/t3.micro, replicas: 3 }
  db: { type: database, flavor: postgres, hosting: managed, instance: aws/db.t3.micro }
  db2: { type: database, flavor: postgres, hosting: managed, instance: aws/db.t3.micro, multi_az: true }
"),
        &BTreeMap::new(),
    );
    let app = cost_of(&c, "app");
    assert_eq!((app.copies, app.unit_micro_usd_hr), (3, Some(10_400)));
    assert_eq!(app.total_micro_usd_hr, Some(31_200));
    assert_eq!(cost_of(&c, "db").total_micro_usd_hr, Some(18_000));
    assert_eq!(
        cost_of(&c, "db2").total_micro_usd_hr,
        Some(18_000 * MULTI_AZ_FACTOR)
    );
    assert_eq!(MULTI_AZ_FACTOR, 2);
    assert_eq!(c.total_micro_usd_hr, 31_200 + 18_000 + 36_000);
    assert_eq!((c.priced, c.unpriced), (3, 0));
    assert_eq!(c.regions["aws"], "us-east-1");
}

/// The region comes from the file; a cache is priced for its engine, so Valkey is 20% below Redis.
#[test]
fn the_files_region_and_a_caches_engine_choose_the_price() {
    let y = |region: &str, flavor: &str| {
        Config::from_yaml(&format!(
            "version: 1\nregion: {region}\nnodes:\n  c: {{ type: cache, flavor: {flavor}, hosting: managed, instance: aws/cache.m5.large }}\n"
        ))
        .unwrap()
    };
    let one = |region: &str, flavor: &str| {
        estimate(&y(region, flavor), &BTreeMap::new()).nodes[0].total_micro_usd_hr
    };
    assert_eq!(one("{ aws: us-east-1 }", "redis"), Some(156_000));
    assert_eq!(one("{ aws: eu-west-1 }", "redis"), Some(172_000));
    assert_eq!(one("{ aws: us-east-1 }", "valkey"), Some(124_800));
    assert_eq!(
        estimate(&y("{ aws: eu-west-1 }", "redis"), &BTreeMap::new()).regions["aws"],
        "eu-west-1"
    );
}

/// S3 Standard is $0.023 a GB-month in us-east-1 (AWS price list): 500 GB over 730 hours, and a Google
/// multi-region store is priced by its `location:`; requests follow the visits a second.
#[test]
fn an_object_store_costs_its_data_over_a_month_plus_requests() {
    let c = cfg("  s3: { type: storage, flavor: s3, stored: 500GB }
  g: { type: storage, flavor: gcs, stored: 100GB, location: nam4 }
  idle: { type: storage, flavor: s3 }
");
    let none = estimate(&c, &BTreeMap::new());
    assert_eq!(
        cost_of(&none, "s3").total_micro_usd_hr,
        Some(15_753) // 500 x 23,000 / 730 = 15,753.4
    );
    // nam4 bills both regions: $0.044 a GB-month
    assert_eq!(
        cost_of(&none, "g").total_micro_usd_hr,
        Some(6_027) // 100 x 44,000 / 730 = 6,027.4
    );
    assert!(cost_of(&none, "idle").why.unwrap().contains("set stored"));
    // 100 reads a second at $0.0004 per 1,000: 360,000 an hour = $0.144 = 144,000 micro
    let busy = estimate(&c, &BTreeMap::from([("idle".to_string(), 100.0)]));
    assert_eq!(cost_of(&busy, "idle").total_micro_usd_hr, Some(144_000));
}

#[test]
fn what_has_no_price_says_why_and_is_left_out_of_the_total() {
    let c = estimate(
        &Config::from_yaml(
            "version: 1
instances:
  rack: { cores: 4 }
nodes:
  mine: { type: service, proc: 1ms, instance: rack }
  lambda: { type: service, proc: 1ms, hosting: serverless }
  idp: { type: auth, hosting: external }
  bare: { type: service, proc: 1ms }
  pick: { type: database, flavor: postgres, hosting: managed }
  gap: { type: cache, flavor: redis, hosting: managed, instance: aws/cache.r8gd.xlarge }
",
        )
        .unwrap(),
        &BTreeMap::new(),
    );
    let why = |n: &str| cost_of(&c, n).why.unwrap_or_default();
    assert!(why("mine").contains("cost_per_hour"));
    assert!(why("lambda").starts_with("serverless"));
    assert!(why("idp").starts_with("run by someone else"));
    assert_eq!(why("bare"), "no instance type");
    assert_eq!(why("pick"), "pick a managed database class");
    // r8gd is sold with Valkey only: a Redis cache has no price for it
    assert!(why("gap").starts_with("no price in us-east-1"));
    assert_eq!((c.priced, c.unpriced, c.total_micro_usd_hr), (0, 6, 0));
}

/// Your own instance is priced by its `cost_per_hour`, in whole micro-USD.
#[test]
fn your_own_instance_is_priced_by_its_cost_per_hour() {
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  rack: { cores: 8, cost_per_hour: 0.25 }\nnodes:\n  a: { type: service, proc: 1ms, instance: rack, replicas: 2 }\n",
    )
    .unwrap();
    let r = estimate(&c, &BTreeMap::new());
    assert_eq!(r.nodes[0].unit_micro_usd_hr, Some(250_000));
    assert_eq!(r.total_micro_usd_hr, 500_000);
}

/// The report is the JSON the run record carries.
#[test]
fn the_report_serializes_for_the_run_record() {
    let c = Config::from_yaml(
        "version: 1\nnodes:\n  a: { type: service, proc: 1ms, instance: aws/t3.micro }\n",
    )
    .unwrap();
    let v = serde_json::to_value(estimate(&c, &BTreeMap::new())).unwrap();
    assert_eq!(v["total_micro_usd_hr"], 10_400);
    assert_eq!(v["nodes"][0]["unit_micro_usd_hr"], 10_400);
    assert_eq!(v["regions"]["gcp"], "us-central1");
}

// RULES 127: a managed MongoDB-flavored database is priced as DocumentDB and a managed Neo4j one as
// Neptune, from the engine the flavor implies.
#[test]
fn a_managed_mongodb_or_neo4j_database_is_priced_by_its_own_engine() {
    let c = estimate(
        &cfg("  docs: { type: database, flavor: mongodb, hosting: managed, instance: aws/db.r5.large }
  graph: { type: database, flavor: neo4j, hosting: managed, instance: aws/db.r5.large }
"),
        &BTreeMap::new(),
    );
    assert_eq!(cost_of(&c, "docs").total_micro_usd_hr, Some(277_000));
    assert_eq!(cost_of(&c, "graph").total_micro_usd_hr, Some(348_000));
}

/// RULES 128: a colder class keeps data for less and bills each GB read back. 1 TB in S3 Standard-IA
/// with 100 GB read back a month is (1,024 x 12,500 + 100 x 10,000) / 730 = 18,904 micro-USD an hour,
/// against 1,024 x 23,000 / 730 = 32,263 in Standard; a class a region does not sell says so.
#[test]
fn a_colder_class_costs_less_to_keep_and_bills_what_is_read_back() {
    let c = cfg(
        "  ia: { type: storage, flavor: s3, class: infrequent, stored: 1024GB, retrieved: 100GB }
  std: { type: storage, flavor: s3, stored: 1024GB }
",
    );
    let r = estimate(&c, &BTreeMap::new());
    assert_eq!(cost_of(&r, "ia").total_micro_usd_hr, Some(18_904));
    assert_eq!(cost_of(&r, "std").total_micro_usd_hr, Some(32_263));
    // an Azure region the archive tier is not sold in has no price for it
    let region = kaanal_core::prices::storage_rows()
        .iter()
        .find(|(p, r, _)| {
            *p == "azure"
                && kaanal_core::prices::storage_class_price(
                    "azure",
                    r,
                    kaanal_core::config::StorageClass::Archive,
                )
                .is_none()
        })
        .map(|(_, r, _)| *r)
        .expect("an Azure region without the archive tier");
    let y = format!(
        "version: 1\nregion: {{ azure: {region} }}\nnodes:\n  a: {{ type: storage, flavor: blob, class: archive, stored: 1024GB }}\n"
    );
    let r = estimate(
        &kaanal_core::Config::from_yaml(&y).unwrap(),
        &BTreeMap::new(),
    );
    let why = cost_of(&r, "a").why.unwrap();
    assert!(
        why.contains("archive object-store price") && why.contains(region),
        "{why}"
    );
}
