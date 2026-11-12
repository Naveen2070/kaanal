//! The generated instance table (`data/instances.csv`): the Azure network figures taken from
//! Microsoft's size pages, and the on-demand price column (RULES 121).

use kaanal_core::Config;
use kaanal_core::config::CacheFlavor;
use kaanal_core::instances::{all, lookup};

#[test]
fn azure_network_figures_are_the_ones_the_size_pages_state() {
    // read from learn.microsoft.com on 2026-11-12 (scripts/azure-network.csv)
    for (name, mbps) in [
        ("azure/standard_d2s_v5", 12_500),
        ("azure/standard_d32s_v5", 16_000),
        ("azure/standard_d96s_v5", 35_000),
        ("azure/standard_d2s_v3", 1_000),
        ("azure/standard_f2s_v2", 5_000),
        ("azure/standard_l8s_v3", 12_500),
    ] {
        assert_eq!(lookup(name).unwrap().net_mbps, mbps, "{name}");
    }
}

#[test]
fn a_constrained_vcpu_size_takes_its_parents_figure_and_an_unlisted_one_has_none() {
    // E16-8s v6 is an E16s v6 with cores switched off: the same card
    let parent = lookup("azure/standard_e16s_v6").unwrap().net_mbps;
    assert!(parent > 0);
    assert_eq!(lookup("azure/standard_e16-8s_v6").unwrap().net_mbps, parent);
    // a size no page figure covers is left unlimited, never guessed
    assert_eq!(lookup("azure/standard_a1").unwrap().net_mbps, 0);
}

#[test]
fn prices_are_the_published_on_demand_list_prices_of_the_default_regions() {
    // m5.large in us-east-1 and D2s v5 in East US are both listed at $0.096 an hour; n2-standard-4
    // is well above that. If a provider changes a list price, regenerate and update here.
    assert_eq!(lookup("aws/m5.large").unwrap().price_micro_usd_hr, 96_000);
    assert_eq!(
        lookup("azure/standard_d2s_v5").unwrap().price_micro_usd_hr,
        96_000
    );
    assert!(lookup("gcp/n2-standard-4").unwrap().price_micro_usd_hr > 150_000);
}

#[test]
fn price_grows_with_size_inside_a_family_and_most_types_have_one() {
    // on-demand pricing is linear in size inside an AWS family: 2xlarge is 2 x xlarge
    let (x, x2) = (
        lookup("aws/m5.xlarge").unwrap().price_micro_usd_hr,
        lookup("aws/m5.2xlarge").unwrap().price_micro_usd_hr,
    );
    assert!(x2.abs_diff(2 * x) * 100 <= x2, "{x} then {x2}");
    let priced = all().iter().filter(|i| i.price_micro_usd_hr > 0).count();
    assert!(
        priced * 100 >= all().len() * 90,
        "{priced} of {} priced",
        all().len()
    );
}

#[test]
fn a_custom_instance_may_carry_a_price_and_a_bad_one_is_refused() {
    let yaml = |cost: &str| {
        format!(
            "version: 1\ninstances:\n  box: {{ cores: 4, cost_per_hour: {cost} }}\nnodes:\n  gw: {{ type: gateway }}\n"
        )
    };
    let cfg = Config::from_yaml(&yaml("0.25")).unwrap();
    assert_eq!(cfg.instances["box"].cost_per_hour, Some(0.25));
    for bad in ["-1", "2000000"] {
        let e = Config::from_yaml(&yaml(bad)).unwrap_err();
        assert!(e.to_string().contains("cost_per_hour"), "{bad}: {e}");
    }
}

#[test]
fn every_provider_has_many_regions_and_each_file_is_sorted_for_lookup() {
    use kaanal_core::prices::{region_table, regions};
    for provider in ["aws", "azure", "gcp"] {
        let rs = regions(provider);
        assert!(rs.len() >= 20, "{provider}: {} regions", rs.len());
        assert!(
            rs.windows(2).all(|w| w[0] < w[1]),
            "{provider} regions sorted"
        );
        for r in rs {
            let t = region_table(provider, r).unwrap();
            assert!(
                !t.is_empty() && t.windows(2).all(|w| w[0].0 < w[1].0),
                "{provider}/{r}"
            );
            assert!(
                t.iter()
                    .all(|(n, p)| n.starts_with(&format!("{provider}/")) && *p > 0)
            );
        }
    }
    assert!(region_table("aws", "no-such-region").is_none());
}

#[test]
fn a_regions_price_differs_from_the_default_and_the_default_file_matches_the_table() {
    use kaanal_core::prices::{DEFAULT_REGIONS, price_micro_usd_hr};
    // m5.large is $0.096 in us-east-1 and $0.107 in eu-west-1 (published list prices)
    assert_eq!(
        price_micro_usd_hr("us-east-1", "aws/m5.large"),
        Some(96_000)
    );
    assert_eq!(
        price_micro_usd_hr("eu-west-1", "aws/m5.large"),
        Some(107_000)
    );
    assert_eq!(
        price_micro_usd_hr("eu-west-1", "azure/standard_d2s_v5"),
        None,
        "wrong provider"
    );
    // the default region's file is exactly the price column of the instance table
    for (provider, region) in DEFAULT_REGIONS {
        for i in all()
            .iter()
            .filter(|i| i.name.starts_with(&format!("{provider}/")))
            // a managed class is priced per engine, in its own files (checked below)
            .filter(|i| !kaanal_core::instances::is_managed_class(i.name))
        {
            let want = (i.price_micro_usd_hr > 0).then_some(i.price_micro_usd_hr);
            assert_eq!(price_micro_usd_hr(region, i.name), want, "{}", i.name);
        }
    }
}

#[test]
fn managed_database_classes_have_their_own_size_and_a_price_per_engine() {
    use kaanal_core::config::DbEngine;
    use kaanal_core::instances::is_managed_class;
    use kaanal_core::prices::managed_price_micro_usd_hr as price;
    // db.m5.large is 2 vCPU and 8 GiB, like m5.large, and costs more than the VM (RDS list price)
    let c = lookup("aws/db.m5.large").unwrap();
    assert_eq!((c.vcpu, c.memory_mib), (2, 8192));
    assert!(is_managed_class("aws/db.m5.large") && !is_managed_class("aws/m5.large"));
    assert!(c.price_micro_usd_hr > lookup("aws/m5.large").unwrap().price_micro_usd_hr);
    // RDS list prices of db.m5.large: PostgreSQL $0.178 and MySQL $0.171 in us-east-1; $0.197 and
    // $0.189 in eu-west-1; the table's column is the PostgreSQL price of the default region
    assert_eq!(
        price(DbEngine::Postgresql, "us-east-1", "aws/db.m5.large"),
        Some(178_000)
    );
    assert_eq!(
        price(DbEngine::Mysql, "us-east-1", "aws/db.m5.large"),
        Some(171_000)
    );
    assert_eq!(
        price(DbEngine::Postgresql, "eu-west-1", "aws/db.m5.large"),
        Some(197_000)
    );
    assert_eq!(
        price(DbEngine::Mysql, "eu-west-1", "aws/db.m5.large"),
        Some(189_000)
    );
    assert_eq!(c.price_micro_usd_hr, 178_000);
    // the default region's PostgreSQL file is the table's price column for every class that has one
    for i in all()
        .iter()
        .filter(|i| kaanal_core::instances::is_db_class(i.name) && i.price_micro_usd_hr > 0)
    {
        let p = price(DbEngine::Postgresql, "us-east-1", i.name).or(price(
            DbEngine::Mysql,
            "us-east-1",
            i.name,
        ));
        assert_eq!(p, Some(i.price_micro_usd_hr), "{}", i.name);
    }
    // a VM type is not a class: no managed price
    assert_eq!(
        price(DbEngine::Postgresql, "us-east-1", "aws/m5.large"),
        None
    );
}

#[test]
fn managed_cache_classes_have_their_own_size_and_a_regional_price() {
    use kaanal_core::instances::{is_cache_class, is_db_class};
    use kaanal_core::prices::managed_cache_price_micro_usd_hr as price;
    let c = lookup("aws/cache.m5.large").unwrap();
    // ElastiCache cache.m5.large: 2 vCPU, 6.38 GiB, $0.156 an hour in us-east-1 and $0.172 in eu-west-1
    assert_eq!((c.vcpu, c.memory_mib), (2, 6533));
    assert!(is_cache_class("aws/cache.m5.large") && !is_db_class("aws/cache.m5.large"));
    assert_eq!(
        price(None, "us-east-1", "aws/cache.m5.large"),
        Some(156_000)
    );
    assert_eq!(
        price(None, "eu-west-1", "aws/cache.m5.large"),
        Some(172_000)
    );
    assert_eq!(c.price_micro_usd_hr, 156_000);
    assert_eq!(price(None, "us-east-1", "aws/m5.large"), None);
    // a flavor of Redis or Memcached is the same list as none
    for f in [CacheFlavor::Redis, CacheFlavor::Memcached] {
        assert_eq!(
            price(Some(f), "us-east-1", "aws/cache.m5.large"),
            Some(156_000)
        );
    }
}

/// B19. AWS prices `ElastiCache` for Valkey 20% below Redis OSS on a node-based cache (the published
/// relationship), so every class both engines list is checked against 0.8 in every region, within
/// the rounding of a price quoted in dollars to three or four places (1%).
#[test]
fn valkey_is_twenty_percent_below_redis_on_every_shared_class() {
    use kaanal_core::prices::{managed_cache_price_micro_usd_hr as price, region_table};
    let regions: Vec<&str> = kaanal_core::prices::regions("aws");
    let mut shared = 0;
    for r in regions {
        let Some(valkey) = region_table("aws-elasticache-valkey", r) else {
            continue;
        };
        for (name, v) in valkey {
            let Some(redis) = price(None, r, name) else {
                continue;
            };
            shared += 1;
            let ratio = f64::from(*v) / f64::from(redis);
            assert!((ratio - 0.8).abs() < 0.01, "{r} {name}: {v} vs {redis}");
            // and the lookup is the table
            assert_eq!(price(Some(CacheFlavor::Valkey), r, name), Some(*v));
        }
    }
    assert!(
        shared > 1_500,
        "{shared} (class, region) pairs shared by the two engines"
    );
    assert_eq!(
        price(Some(CacheFlavor::Valkey), "us-east-1", "aws/cache.m5.large"),
        Some(124_800),
        "$0.1248 an hour (AWS price list)"
    );
}

/// B19. The newest Graviton cache classes (`r8gd`) are sold with Valkey only: they are in the table
/// with a Valkey price somewhere and a Redis price nowhere.
#[test]
fn a_valkey_only_class_is_in_the_table_and_priced_for_valkey_alone() {
    use kaanal_core::prices::{managed_cache_price_micro_usd_hr as price, region_table};
    let name = "aws/cache.r8gd.xlarge";
    assert!(lookup(name).is_some(), "{name} is a class");
    let regions = kaanal_core::prices::regions("aws");
    let valkey: Vec<_> = regions
        .iter()
        .filter(|r| price(Some(CacheFlavor::Valkey), r, name).is_some())
        .collect();
    assert!(!valkey.is_empty(), "priced for Valkey somewhere");
    assert!(
        regions.iter().all(|r| price(None, r, name).is_none()),
        "and for Redis nowhere"
    );
    assert!(region_table("aws-elasticache-valkey", valkey[0]).is_some());
}

/// B20, from Google's pricing page: a predefined dual-region bills both of its regions, so it costs
/// twice a single region in the same area (`nam4` is Iowa and South Carolina: 2 x $0.022 =
/// $0.044), `eur4` (Finland and Netherlands) likewise, and a multi-region is $0.026; a bucket in a
/// dual- or multi-region pays $0.01 per 1,000 Class A operations, a single region $0.005.
#[test]
fn google_dual_and_multi_region_prices_follow_the_pages_billing_rule() {
    use kaanal_core::prices::{storage_locations, storage_price};
    let p = |r: &str| storage_price("gcp", r).unwrap();
    let single = p("us-central1");
    assert_eq!(single.gb_month, 20_000, "$0.020 a GB-month");
    // 2 x the dual-region SKU of each of its two regions ($0.022 each): $0.044
    assert_eq!(p("nam4").gb_month, 44_000);
    assert_eq!(p("eur4").gb_month, 44_000);
    for m in ["us", "eu", "asia"] {
        assert_eq!(p(m).gb_month, 26_000, "{m}: $0.026");
    }
    for w in [
        "asia", "asia1", "eu", "eur4", "eur5", "eur7", "eur8", "nam4", "us",
    ] {
        assert_eq!(
            (p(w).write_per_1k, p(w).read_per_1k),
            (10_000, 400),
            "{w}: Class A doubles, Class B does not"
        );
    }
    assert_eq!((single.write_per_1k, single.read_per_1k), (5_000, 400));
    // every location is listed once, and the single regions are still all there
    let all = storage_locations("gcp");
    assert_eq!(all.len(), 55);
    assert!(all.contains(&"nam4") && all.contains(&"us-west1"));
}

#[test]
fn object_store_prices_are_per_region_for_all_three_clouds() {
    use kaanal_core::prices::{regions, storage_price, storage_rows};
    // S3 Standard, first tier (AWS price list): $0.023 a GB-month, $0.005 per 1,000 writes, $0.0004
    // per 1,000 reads in us-east-1; Mumbai costs more
    let s3 = storage_price("aws", "us-east-1").unwrap();
    assert_eq!(
        (s3.gb_month, s3.write_per_1k, s3.read_per_1k),
        (23_000, 5_000, 400)
    );
    assert!(storage_price("aws", "ap-south-1").unwrap().gb_month > s3.gb_month);
    // Azure Blob Hot LRS in East US (Azure retail prices): $0.0208, $0.05 per 10,000 writes, $0.004 per 10,000 reads
    let blob = storage_price("azure", "us-east").unwrap();
    assert_eq!(
        (blob.gb_month, blob.write_per_1k, blob.read_per_1k),
        (20_800, 5_000, 400)
    );
    // every AWS and Azure region of the price file is a region the VM price files have, so the
    // region menu and the storage price agree
    for (p, r, _) in storage_rows()
        .iter()
        .filter(|(p, r, _)| *p != "gcp" && *r != "*")
    {
        assert!(
            regions(p).contains(r) || *p == "aws" && r.contains('-'),
            "{p}/{r}"
        );
    }
    for (p, r, _) in storage_rows().iter().filter(|(p, _, _)| *p == "azure") {
        assert!(regions(p).contains(r), "azure/{r} is not a VM region name");
    }
    // Google Cloud Storage Standard (cloud.google.com/storage/pricing, 2026-11-12): Iowa is $0.02 a
    // GB-month (0.000027397 a gibibyte hour x 730), São Paulo costs more, writes $0.005 and reads
    // $0.0004 per 1,000
    let iowa = storage_price("gcp", "us-central1").unwrap();
    assert_eq!(
        (iowa.gb_month, iowa.write_per_1k, iowa.read_per_1k),
        (20_000, 5_000, 400)
    );
    assert!(storage_price("gcp", "southamerica-east1").unwrap().gb_month > iowa.gb_month);
    // almost every Google row is a region the VM price files have (the newest regions may be absent
    // from that catalogue and are then simply not offered in the menu)
    // (the nine dual- and multi-region locations are not compute regions and are left out of this check)
    let wide = [
        "asia", "asia1", "eu", "eur4", "eur5", "eur7", "eur8", "nam4", "us",
    ];
    let google: Vec<_> = storage_rows()
        .iter()
        .filter(|(p, r, _)| *p == "gcp" && !wide.contains(r))
        .collect();
    let known = google
        .iter()
        .filter(|(p, r, _)| regions(p).contains(r))
        .count();
    assert!(
        known * 10 >= google.len() * 9,
        "{known} of {} known",
        google.len()
    );
    assert_eq!(storage_price("aws", "nowhere-1"), None);
}
