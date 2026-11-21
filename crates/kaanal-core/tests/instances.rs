//! Series and ladder of the instance catalogue (step 9c-1, Task 1).

use kaanal_core::{instances, prices};

#[test]
fn series_of_the_three_naming_styles() {
    let cases = [
        ("aws/m5.large", "aws/m5"),
        ("aws/db.m5.xlarge", "aws/db.m5"),
        ("gcp/n2-standard-8", "gcp/n2-standard"),
        ("azure/standard_d4s_v5", "azure/standard_d#s_v5"),
        ("azure/db.gp_gen5_4", "azure/db.gp_gen5"),
        ("gcp/c3d-highmem-360-lssd", "gcp/c3d-highmem-lssd"),
    ];
    for (name, want) in cases {
        assert_eq!(instances::series(name).as_deref(), Some(want), "{name}");
    }
}

#[test]
fn every_catalogue_class_has_a_series_or_is_listed_as_an_exception() {
    let none: Vec<&str> = instances::all()
        .iter()
        .filter(|i| instances::series(i.name).is_none())
        .map(|i| i.name)
        .collect();
    // GCP shared-core e2 sizes carry a word, not a number; a new odd name fails here loudly.
    let expected = ["gcp/e2-medium", "gcp/e2-micro", "gcp/e2-small"];
    assert_eq!(none, expected);
}

#[test]
fn ladder_is_sorted_by_vcpu_then_price_and_holds_only_priced_classes() {
    let l = instances::ladder("aws/m5.large", "us-east-1");
    let names: Vec<&str> = l.iter().map(|i| i.name).collect();
    assert!(names.contains(&"aws/m5.large") && names.contains(&"aws/m5.xlarge"));
    assert!(l.windows(2).all(|w| w[0].vcpu <= w[1].vcpu));
    assert!(
        l.iter()
            .all(|i| prices::price_micro_usd_hr("us-east-1", i.name).is_some())
    );
    assert!(
        l.iter()
            .all(|i| instances::series(i.name).as_deref() == Some("aws/m5"))
    );
}

#[test]
fn a_class_with_no_series_has_an_empty_ladder() {
    let none: Vec<&instances::Instance> = Vec::new();
    assert_eq!(instances::ladder("gcp/e2-micro", "us-central1"), none);
    assert_eq!(instances::ladder("nonsense", "us-east-1"), none);
}
