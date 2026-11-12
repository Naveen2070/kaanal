//! The generated instance table (`data/instances.csv`): the Azure network figures taken from
//! Microsoft's size pages, and the on-demand price column (RULES 121).

use kaanal_core::Config;
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
        {
            let want = (i.price_micro_usd_hr > 0).then_some(i.price_micro_usd_hr);
            assert_eq!(price_micro_usd_hr(region, i.name), want, "{}", i.name);
        }
    }
}
