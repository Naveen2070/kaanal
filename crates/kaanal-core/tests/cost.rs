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

/// AWS prices the first tier of data out to the internet at $0.09 a GB in us-east-1 (price list,
/// `AWSDataTransfer`) with 100 GB a month free; Azure's global network is $0.087 with 100 GB free;
/// Google's headline is $0.12. The oracle is that arithmetic, 730 hours to the month.
#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn what_a_run_sends_out_is_priced_per_cloud_less_the_free_allowance() {
    use kaanal_core::cost::{EgressFlow, estimate_with_egress};
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let c = cfg("  app: { type: service, proc: 1ms, instance: aws/t3.micro }\n");
    let flow = |p: &str, bytes_per_s: f64| EgressFlow {
        provider: p.to_string(),
        bytes_per_s,
    };
    let gb_month = |bytes_per_s: f64| bytes_per_s * 3600.0 * 730.0 / GIB;
    let hourly = |bytes_per_s: f64, free: f64, per_gb: f64| {
        ((gb_month(bytes_per_s) - free).max(0.0) * per_gb / 730.0).round() as u64
    };
    let r = estimate_with_egress(
        &c,
        &BTreeMap::new(),
        &[flow("aws", GIB), flow("azure", 1.0e6), flow("gcp", 1.0e6)],
    );
    let by = |p: &str| r.egress.iter().find(|e| e.provider == p).unwrap().clone();
    assert_eq!(by("aws").micro_usd_hr, Some(hourly(GIB, 100.0, 90_000.0)));
    assert_eq!(
        by("azure").micro_usd_hr,
        Some(hourly(1.0e6, 100.0, 87_000.0))
    );
    assert_eq!(by("gcp").micro_usd_hr, Some(hourly(1.0e6, 0.0, 120_000.0)));
    assert_eq!(by("aws").bytes_per_s, 1_073_741_824);
    // the fees are in the total with the node
    let fees: u64 = r.egress.iter().filter_map(|e| e.micro_usd_hr).sum();
    assert_eq!(r.total_micro_usd_hr, 10_400 + fees);
    // a trickle inside the free allowance costs nothing
    let small = estimate_with_egress(&c, &BTreeMap::new(), &[flow("aws", 1000.0)]);
    assert_eq!(small.egress[0].micro_usd_hr, Some(0));
    // no cloud on the entry node: AWS's rate, said so
    let own = estimate_with_egress(&c, &BTreeMap::new(), &[flow("", GIB)]);
    assert_eq!(own.egress[0].provider, "aws");
    assert!(own.egress[0].note.as_deref().unwrap().contains("no cloud"));
    assert_eq!(own.egress[0].micro_usd_hr, by("aws").micro_usd_hr);
    // no run, no egress line
    assert_eq!(estimate(&c, &BTreeMap::new()).egress, []);
}

/// A stream is billed in the cloud of its entry node (the gateway), whatever the cloud of the rest.
#[test]
fn a_streams_bytes_go_to_the_cloud_of_its_entry_node() {
    use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
    use kaanal_core::cost::egress_flows;
    use kaanal_core::{Config, Topology};
    let cfg = Config::from_yaml(
        "version: 1
nodes:
  gw: { type: gateway, proc: 1ms, instance: azure/standard_d2s_v5 }
  app: { type: service, proc: 1ms, instance: aws/t3.micro }
edges:
  - { from: gw, to: app }
endpoints:
",
    )
    .unwrap();
    let t = Topology::build(&cfg).unwrap();
    let specs = [RouteSpec::inferred("GET /a", t.id("app").unwrap())];
    let eps = compile(&t, &specs, EndpointsInput::inline(&cfg)).unwrap();
    let flows = egress_flows(&cfg, &t, &eps, [("GET /a", 500.0), ("GET /nope", 9.0)]);
    assert_eq!(flows.len(), 1);
    assert_eq!(
        (flows[0].provider.as_str(), flows[0].bytes_per_s),
        ("azure", 500.0)
    );
}

/// AWS list prices, us-east-1: Lambda $0.20 a million requests and $0.0000166667 a GB-second (x86), API
/// Gateway HTTP API $1.00 a million (REST $3.50). 100 requests a second for an hour is 360,000 requests; at 100 ms and 512 MB
/// that is 18,000 GB-seconds. The oracle is that arithmetic (the price file rounds a GB-second to 16.667
/// micro-USD, so the Lambda figure is held to 10 micro-USD).
#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn serverless_and_a_managed_gateway_are_priced_by_use_from_the_visits() {
    let c = cfg("  fn: { type: service, proc: 100ms, hosting: serverless, memory: { limit: 512MB, per_request: 1MB } }
  small: { type: service, proc: 100ms, hosting: serverless }
  gw: { type: gateway, proc: 1ms, hosting: managed }
");
    let visits = BTreeMap::from([
        ("fn".to_string(), 100.0),
        ("small".to_string(), 100.0),
        ("gw".to_string(), 100.0),
    ]);
    let r = estimate(&c, &visits);
    let lambda = cost_of(&r, "fn");
    let want = 72_000.0 + 18_000.0 * 16.6667; // requests + GB-seconds, micro-USD an hour
    assert!(
        (lambda.total_micro_usd_hr.unwrap() as f64 - want).abs() <= 10.0,
        "{lambda:?}"
    );
    assert!(lambda.note.as_deref().unwrap().contains("AWS Lambda"));
    // no memory: set, so Lambda's 128 MB: a quarter of the GB-seconds
    let small = cost_of(&r, "small");
    let want = 72_000.0 + 4_500.0 * 16.6667;
    assert!(
        (small.total_micro_usd_hr.unwrap() as f64 - want).abs() <= 10.0,
        "{small:?}"
    );
    assert!(small.note.as_deref().unwrap().contains("128 MB"));
    // an unnamed API is an HTTP API (RULES 133 a): 360,000 requests at $1.00 a million
    assert_eq!(cost_of(&r, "gw").total_micro_usd_hr, Some(360_000));
    assert_eq!((r.priced, r.unpriced), (3, 0));
    // without visits they say what they need, and stay out of the total
    let none = estimate(&c, &BTreeMap::new());
    assert!(
        cost_of(&none, "fn")
            .why
            .unwrap()
            .starts_with("serverless: priced by use")
    );
    assert!(
        cost_of(&none, "gw")
            .why
            .unwrap()
            .starts_with("managed: priced by use")
    );
    assert_eq!(none.total_micro_usd_hr, 0);
}

/// RULES 133 a: a managed gateway is an API Gateway HTTP API unless it says `api: rest`; the note says which
/// was assumed, and `api:` is refused on a gateway that is not `hosting: managed`.
#[test]
fn a_managed_gateway_is_an_http_api_unless_it_says_rest() {
    let visits = BTreeMap::from([("gw".to_string(), 100.0)]);
    let one = |props: &str| {
        let c = cfg(&format!(
            "  gw: {{ type: gateway, hosting: managed{props} }}
"
        ));
        cost_of(&estimate(&c, &visits), "gw")
    };
    // 100 req/s for an hour is 360,000 requests: $1.00 a million (HTTP API) or $3.50 (REST API)
    let default = one("");
    assert_eq!(default.total_micro_usd_hr, Some(360_000));
    assert!(
        default
            .note
            .as_deref()
            .unwrap()
            .contains("HTTP API (write api: rest"),
        "{default:?}"
    );
    let http = one(", api: http");
    assert_eq!(http.total_micro_usd_hr, Some(360_000));
    let note = http.note.as_deref().unwrap();
    assert!(
        note.contains("an API Gateway HTTP API") && !note.contains("write api"),
        "{http:?}"
    );
    let rest = one(", api: rest");
    assert_eq!(rest.total_micro_usd_hr, Some(1_260_000));
    assert!(
        rest.note
            .as_deref()
            .unwrap()
            .contains("an API Gateway REST API"),
        "{rest:?}"
    );
    // the field belongs to a managed gateway
    let e = Config::from_yaml(
        "version: 1
nodes:
  gw: { type: gateway, api: rest }
",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("api goes with a gateway that has hosting: managed"),
        "{e}"
    );
    let e = Config::from_yaml(
        "version: 1
nodes:
  s: { type: service, hosting: serverless, api: rest }
",
    )
    .unwrap_err();
    // another kind does not have the field at all
    assert!(e.to_string().contains("unknown field 'api'"), "{e}");
    assert!(
        Config::from_yaml(
            "version: 1
nodes:
  gw: { type: gateway, api: bogus, hosting: managed }
"
        )
        .is_err()
    );
}

/// RULES 133 b: a Multi-AZ licence-included Oracle or SQL Server database costs AWS's own Multi-AZ price for
/// its class and region (the standby runs 1.23 to 2.59 times the instance, not always 2). The oracle is the
/// two price files read here directly, not the lookup under test; where a class has no Multi-AZ row (SQL
/// Server Express has none) the price is twice the Single-AZ one and the note says so.
#[test]
fn a_licensed_multi_az_database_reads_the_real_standby_price() {
    let read = |provider: &str, region: &str| -> BTreeMap<String, u64> {
        let path = format!(
            "{}/data/prices/{provider}/{region}.csv",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (n, p) = l.split_once(',').unwrap();
                (n.to_string(), p.parse().unwrap())
            })
            .collect()
    };
    let est = |region: &str, engine: &str, class: &str, multi: bool| {
        let flavor = if engine.starts_with("sqlserver") {
            "sqlserver"
        } else {
            "oracle"
        };
        let c = Config::from_yaml(&format!(
            "version: 1
region: {{ aws: {region} }}
nodes:
  db: {{ type: database, flavor: {flavor}, hosting: managed, engine: {engine}, instance: aws/{class}, multi_az: {multi} }}
"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        cost_of(&estimate(&c, &BTreeMap::new()), "db")
    };
    let single = read("aws-rds-oracle-se2", "us-east-1");
    let multi = read("aws-rds-oracle-se2-multiaz", "us-east-1");
    // a class whose standby is not exactly twice the instance, so a flat factor of 2 would be wrong
    let (class, (s, m)) = single
        .iter()
        .filter_map(|(n, s)| multi.get(n).map(|m| (n, (*s, *m))))
        .find(|(_, (s, m))| m.abs_diff(2 * s) * 100 > 2 * s)
        .expect("the price list has a class whose Multi-AZ price is not 2 x");
    let class = class.trim_start_matches("aws/");
    assert_eq!(
        est("us-east-1", "oracle-se2", class, false).total_micro_usd_hr,
        Some(s)
    );
    let got = est("us-east-1", "oracle-se2", class, true);
    assert_eq!(got.total_micro_usd_hr, Some(m), "{got:?}");
    assert_ne!(Some(m), Some(2 * s));
    assert!(got.note.is_none(), "{got:?}");
    // a class with a Single-AZ price and no Multi-AZ row falls back to 2 x, with a note
    let ex = read("aws-rds-sqlserver-ex", "us-east-1");
    let (name, p) = ex.iter().next().unwrap();
    let fell = est(
        "us-east-1",
        "sqlserver-ex",
        name.trim_start_matches("aws/"),
        true,
    );
    assert_eq!(fell.total_micro_usd_hr, Some(2 * p), "{fell:?}");
    assert!(
        fell.note
            .as_deref()
            .unwrap()
            .contains("no Multi-AZ price for this class"),
        "{fell:?}"
    );
}
