//! Provider translation (Step 9c-2, plan Task 1): the oracle is a scan of `instances::all()` and the
//! region's price file, never a figure the code printed.

use kaanal_core::Config;
use kaanal_core::instances::{self, Instance};
use kaanal_core::prices;
use kaanal_core::resize::{providers_of, translate};

fn cfg(nodes: &str) -> Config {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
}

/// The region a translation to `to` prices in when the file writes none: the cloud's default.
fn region(to: &str) -> &'static str {
    prices::DEFAULT_REGIONS
        .iter()
        .find(|(p, _)| *p == to)
        .map(|(_, r)| *r)
        .unwrap()
}

/// The oracle: every priced compute class of `to` at least as big as `from` (network compared only
/// when both give one), cheapest first.
fn fits(from: &Instance, to: &str, compare_net: bool) -> Vec<(u32, &'static str)> {
    let prefix = format!("{to}/");
    let mut v: Vec<(u32, &'static str)> = instances::all()
        .iter()
        .filter(|i| i.name.starts_with(&prefix) && !instances::is_managed_class(i.name))
        .filter(|i| i.vcpu >= from.vcpu && i.memory_mib >= from.memory_mib)
        .filter(|i| {
            !compare_net || i.net_mbps == 0 || from.net_mbps == 0 || i.net_mbps >= from.net_mbps
        })
        .filter_map(|i| prices::price_micro_usd_hr(region(to), i.name).map(|p| (p, i.name)))
        .collect();
    v.sort_unstable();
    v
}

fn class_of(c: &Config, node: &str) -> &'static Instance {
    instances::lookup(c.nodes[node].instance_name().unwrap()).unwrap()
}

#[test]
fn providers_of_lists_the_providers_of_table_classes() {
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  big: { cores: 8, cost_per_hour: 1.5 }\nnodes:\n  s: { type: service, proc: 5ms, instance: gcp/n2-standard-2 }\n  t: { type: service, proc: 5ms, instance: aws/m5.large }\n  u: { type: service, proc: 5ms, instance: big }\n  d: { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.large }\n  e: { type: service, proc: 5ms }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(providers_of(&c), vec!["aws".to_string(), "gcp".to_string()]);
    assert_eq!(
        providers_of(&cfg("  e: { type: service, proc: 5ms }\n")),
        Vec::<String>::new()
    );
}

#[test]
fn a_compute_node_goes_to_the_cheapest_class_at_least_as_big() {
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/m5.large }\n");
    let old = class_of(&c, "s");
    for to in ["gcp", "azure"] {
        let t = translate(&c, to, &[]).unwrap_or_else(|e| panic!("{to}: {e}"));
        let new = class_of(&t.config, "s");
        assert!(new.name.starts_with(&format!("{to}/")), "{}", new.name);
        assert!(
            new.vcpu >= old.vcpu && new.memory_mib >= old.memory_mib,
            "{new:?}"
        );
        let price = prices::price_micro_usd_hr(region(to), new.name).expect("priced");
        let cheapest = fits(old, to, true)[0];
        assert_eq!((price, new.name), cheapest, "{to}");
        assert_eq!(t.config.region, c.region, "region: is not changed");
        t.config.validate_candidate().unwrap();
    }
}

#[test]
fn a_missing_network_rate_is_not_compared() {
    // aws/c3.large gives no network rate: only vCPU and memory decide
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/c3.large }\n");
    let old = class_of(&c, "s");
    assert_eq!(old.net_mbps, 0);
    let t = translate(&c, "gcp", &[]).unwrap();
    assert_eq!(class_of(&t.config, "s").name, fits(old, "gcp", false)[0].1);
    // and a target class that gives none is not ruled out by the original's rate
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/m5.large }\n");
    let t = translate(&c, "azure", &[]).unwrap();
    let all_fit = fits(class_of(&c, "s"), "azure", true);
    assert_eq!(class_of(&t.config, "s").name, all_fit[0].1);
    assert!(
        all_fit
            .iter()
            .any(|(_, n)| instances::lookup(n).unwrap().net_mbps == 0)
    );
}

#[test]
fn a_custom_instance_and_a_kept_node_are_kept_as_written() {
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  big: { cores: 8, cost_per_hour: 1.5 }\nnodes:\n  u: { type: service, proc: 5ms, instance: big }\n  k: { type: service, proc: 5ms, cpu: { cores: 4 } }\n  e: { type: service, proc: 5ms }\n  s: { type: service, proc: 5ms, instance: aws/m5.large }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let t = translate(&c, "gcp", &["k".into()]).unwrap();
    for n in ["u", "k", "e"] {
        assert_eq!(
            t.config.nodes[n].instance_name(),
            c.nodes[n].instance_name(),
            "{n}"
        );
        assert!(
            t.notes
                .iter()
                .any(|x| x.starts_with(&format!("{n}: kept as written, not compared"))),
            "{n}: {:?}",
            t.notes
        );
    }
    assert!(class_of(&t.config, "s").name.starts_with("gcp/"));
}

#[test]
fn a_cache_on_a_class_makes_the_section_skip_instead_of_mixing_providers() {
    // translation has no cache classes of another cloud: moving the service and keeping the cache
    // would price an aws cache beside a gcp service
    let c = cfg(
        "  s: { type: service, proc: 5ms, instance: aws/m5.large }\n  c: { type: cache, hosting: managed, instance: aws/cache.m5.large }\n",
    );
    assert_eq!(
        translate(&c, "gcp", &[]).unwrap_err(),
        "c: a cache on aws has no translation, so a section on gcp would mix providers"
    );
    // a cache with no class prices nothing and stays as written
    let c =
        cfg("  s: { type: service, proc: 5ms, instance: aws/m5.large }\n  c: { type: cache }\n");
    assert!(translate(&c, "gcp", &[]).is_ok());
}

#[test]
fn a_provider_that_cannot_translate_is_skipped_with_the_reason() {
    let c = cfg(
        "  d: { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.large }\n",
    );
    // Azure's managed class is Azure SQL (sqlserver only), and gcp has none: the reason names
    // the flavor and says the price table lacks it, not that no equivalent exists
    let why = translate(&c, "azure", &[]).unwrap_err();
    assert_eq!(
        why,
        "d: no managed postgres class for azure in the price table"
    );
    assert_eq!(
        translate(&c, "gcp", &[]).unwrap_err(),
        "d: no managed postgres class for gcp in the price table"
    );
    let m = cfg(
        "  d: { type: database, flavor: mysql, hosting: managed, instance: aws/db.m5.large }\n",
    );
    assert_eq!(
        translate(&m, "gcp", &[]).unwrap_err(),
        "d: no managed mysql class for gcp in the price table"
    );
    // a provider with no price list
    assert!(translate(&c, "mars", &[]).is_err());
}

#[test]
fn a_managed_sqlserver_goes_to_the_cheapest_azure_sql_class_at_least_as_big() {
    let c = cfg(
        "  d: { type: database, flavor: sqlserver, hosting: managed, engine: sqlserver-standard, instance: aws/db.m5.large }\n",
    );
    let old = class_of(&c, "d");
    let t = translate(&c, "azure", &[]).unwrap();
    let new = class_of(&t.config, "d");
    assert!(instances::is_db_class(new.name), "{}", new.name);
    assert!(new.vcpu >= old.vcpu && new.memory_mib >= old.memory_mib);
    let cheaper = instances::all().iter().filter(|i| {
        i.name.starts_with("azure/db.")
            && i.vcpu >= old.vcpu
            && i.memory_mib >= old.memory_mib
            && (i.net_mbps == 0 || old.net_mbps == 0 || i.net_mbps >= old.net_mbps)
    });
    let price = |n: &str| {
        prices::managed_price_micro_usd_hr(
            kaanal_core::config::DbEngine::AzureSql,
            region("azure"),
            n,
        )
    };
    let mine = price(new.name).expect("priced");
    assert!(cheaper.filter_map(|i| price(i.name)).all(|p| p >= mine));
    assert_eq!(
        t.config.nodes["d"].engine_written(),
        Some(kaanal_core::config::DbEngine::AzureSql)
    );
    t.config.validate_candidate().unwrap();
}

#[test]
fn translating_to_the_provider_in_use_is_refused() {
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/m5.large }\n");
    assert_eq!(translate(&c, "aws", &[]).unwrap_err(), "already on aws");
}

#[test]
fn the_input_config_is_not_changed() {
    let c = cfg(
        "  s: { type: service, proc: 5ms, instance: aws/m5.large }\n  d: { type: database, flavor: sqlserver, hosting: managed, instance: aws/db.m5.large }\n",
    );
    let before = format!("{c:?}");
    let t = translate(&c, "azure", &[]).unwrap();
    assert_eq!(format!("{c:?}"), before);
    assert_ne!(format!("{:?}", t.config), before);
}

#[test]
fn the_target_region_is_the_one_written_else_the_default() {
    // none written: the default (us-central1); written: that region, and region: is untouched
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/m5.large }\n");
    let old = class_of(&c, "s");
    let t = translate(&c, "gcp", &[]).unwrap();
    assert_eq!(region("gcp"), "us-central1");
    assert_eq!(class_of(&t.config, "s").name, fits(old, "gcp", true)[0].1);
    assert!(t.config.region.get("gcp").is_none());
    let w = Config::from_yaml(
        "version: 1\nregion: { gcp: europe-west1 }\nnodes:\n  s: { type: service, proc: 5ms, instance: aws/m5.large }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let t = translate(&w, "gcp", &[]).unwrap();
    let new = class_of(&t.config, "s");
    let price = |n: &str| prices::price_micro_usd_hr("europe-west1", n);
    let mine = price(new.name).expect("priced in the written region");
    let cheapest = instances::all()
        .iter()
        .filter(|i| i.name.starts_with("gcp/") && !instances::is_managed_class(i.name))
        .filter(|i| i.vcpu >= old.vcpu && i.memory_mib >= old.memory_mib)
        .filter(|i| i.net_mbps == 0 || i.net_mbps >= old.net_mbps)
        .filter_map(|i| price(i.name))
        .min()
        .unwrap();
    assert_eq!(mine, cheapest);
    assert_eq!(
        t.config.region.get("gcp").map(String::as_str),
        Some("europe-west1")
    );
}

#[test]
fn a_price_tie_goes_to_the_smaller_class() {
    // the table carries a real tie: two Azure classes fit aws/c8gb.48xlarge at the same lowest price
    // in the default region, with different memory; the smaller one is picked
    let c = cfg("  s: { type: service, proc: 5ms, instance: aws/c8gb.48xlarge }\n");
    let all_fit = fits(class_of(&c, "s"), "azure", true);
    let low = all_fit[0].0;
    let mut tied: Vec<&Instance> = all_fit
        .iter()
        .take_while(|(p, _)| *p == low)
        .map(|(_, n)| instances::lookup(n).unwrap())
        .collect();
    assert!(
        tied.len() >= 2,
        "the table no longer has this tie: {tied:?}"
    );
    tied.sort_by_key(|i| (i.vcpu, i.memory_mib, i.name));
    assert_ne!(
        (tied[0].vcpu, tied[0].memory_mib),
        (tied[1].vcpu, tied[1].memory_mib)
    );
    let t = translate(&c, "azure", &[]).unwrap();
    assert_eq!(class_of(&t.config, "s").name, tied[0].name);
}

/// A kept node with a table class of the provider being left would stay there while the rest moves:
/// a mixed design, which is out of scope (egress and cross-cloud latency are not modelled). The
/// whole translation is refused with the node named, a database or not.
#[test]
fn a_kept_node_with_a_class_of_the_provider_left_refuses_a_mixed_design() {
    let c = cfg(
        "  s: { type: service, proc: 5ms, instance: aws/m5.large }\n  d: { type: database, flavor: sqlserver, hosting: managed, instance: aws/db.m5.large }\n",
    );
    for to in ["gcp", "azure"] {
        assert_eq!(
            translate(&c, to, &["d".into()]).unwrap_err(),
            format!("d: kept on aws, so a section on {to} would mix providers"),
        );
    }
    assert_eq!(
        translate(&c, "azure", &["s".into()]).unwrap_err(),
        "s: kept on aws, so a section on azure would mix providers"
    );
}

/// A kept node with no table class (cores written, a custom instance) has no provider, so keeping it
/// mixes nothing: the rest still translates.
#[test]
fn a_kept_node_with_no_table_class_still_translates() {
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  big: { cores: 8, cost_per_hour: 1.5 }\nnodes:\n  u: { type: service, proc: 5ms, instance: big }\n  k: { type: service, proc: 5ms, cpu: { cores: 2 } }\n  s: { type: service, proc: 5ms, instance: aws/m5.large }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let t = translate(&c, "gcp", &["u".into(), "k".into()]).unwrap();
    assert!(class_of(&t.config, "s").name.starts_with("gcp/"));
    assert!(
        t.notes
            .iter()
            .any(|x| x == "u: kept as written, not compared (kept by you)"),
        "{:?}",
        t.notes
    );
    assert!(
        t.notes
            .iter()
            .any(|x| x == "k: kept as written, not compared (kept by you)"),
        "{:?}",
        t.notes
    );
}
