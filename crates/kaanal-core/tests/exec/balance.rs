//! `pick_replica`: the pure choice of a live replica.

use kaanal_core::{Balance, pick_replica};

const UP3: [bool; 3] = [true; 3];

fn rr(up: &[bool], n: u64) -> Option<u16> {
    pick_replica(
        &Balance::RoundRobin,
        u16::try_from(up.len()).unwrap(),
        up,
        n,
        0,
        0,
        "db",
    )
}

#[test]
fn round_robin_cycles_through_live_replicas() {
    let got: Vec<_> = (0..7).map(|n| rr(&UP3, n).unwrap()).collect();
    assert_eq!(got, [0, 1, 2, 0, 1, 2, 0]);
}

#[test]
fn a_down_replica_is_skipped_and_the_cycle_closes_over_the_rest() {
    let got: Vec<_> = (0..6)
        .map(|n| rr(&[true, false, true], n).unwrap())
        .collect();
    assert_eq!(got, [0, 2, 0, 2, 0, 2]);
}

#[test]
fn the_offset_moves_where_the_cycle_starts() {
    let at = |offset| pick_replica(&Balance::RoundRobin, 3, &UP3, 0, offset, 0, "db");
    assert_eq!(
        (at(0), at(1), at(2), at(3)),
        (Some(0), Some(1), Some(2), Some(0))
    );
}

#[test]
fn all_down_gives_none_for_every_policy() {
    for b in [
        Balance::RoundRobin,
        Balance::Random,
        Balance::Weighted(vec![1, 1]),
    ] {
        assert_eq!(
            pick_replica(&b, 2, &[false, false], 5, 0, 9, "db"),
            None,
            "{b:?}"
        );
    }
}

/// How often each of `replicas` is picked over 20 000 paths.
fn spread(b: &Balance, up: &[bool]) -> Vec<f64> {
    let mut hits = vec![0u32; up.len()];
    for path in 0..20_000u64 {
        let r = pick_replica(b, u16::try_from(up.len()).unwrap(), up, path, 0, path, "db").unwrap();
        hits[usize::from(r)] += 1;
    }
    hits.iter().map(|h| f64::from(*h) / 20_000.0).collect()
}

fn near(got: f64, want: f64) {
    assert!(
        (got - want).abs() <= want * 0.05,
        "{got} is not within 5% of {want}"
    );
}

#[test]
fn random_is_uniform_within_5_percent_over_20000() {
    for share in spread(&Balance::Random, &[true; 4]) {
        near(share, 0.25);
    }
}

#[test]
fn random_never_picks_a_down_replica_and_spreads_over_the_rest() {
    let s = spread(&Balance::Random, &[true, false, true, true]);
    assert_eq!(s[1], 0.0);
    for i in [0, 2, 3] {
        near(s[i], 1.0 / 3.0);
    }
}

#[test]
fn weighted_3_1_1_follows_the_weights_within_5_percent() {
    let s = spread(&Balance::Weighted(vec![3, 1, 1]), &UP3);
    near(s[0], 0.6);
    near(s[1], 0.2);
    near(s[2], 0.2);
}

#[test]
fn a_zero_weight_gets_nothing_and_a_down_replica_renormalises() {
    let s = spread(&Balance::Weighted(vec![0, 1, 1]), &UP3);
    assert_eq!(s[0], 0.0);
    let s = spread(&Balance::Weighted(vec![3, 1, 1]), &[false, true, true]);
    assert_eq!(s[0], 0.0);
    near(s[1], 0.5);
}

#[test]
fn when_only_zero_weight_replicas_are_live_the_pick_falls_back_to_uniform() {
    let s = spread(&Balance::Weighted(vec![5, 0, 0]), &[false, true, true]);
    near(s[1], 0.5);
    near(s[2], 0.5);
}

#[test]
fn same_inputs_same_pick() {
    for b in [Balance::Random, Balance::Weighted(vec![2, 1, 1])] {
        for path in 0..200 {
            let a = pick_replica(&b, 3, &UP3, 4, 1, path, "db");
            assert_eq!(a, pick_replica(&b, 3, &UP3, 4, 1, path, "db"));
        }
    }
}

#[test]
fn a_different_node_name_draws_a_different_stream() {
    let a: Vec<_> = (0..64)
        .map(|p| pick_replica(&Balance::Random, 8, &[true; 8], 0, 0, p, "db"))
        .collect();
    let b: Vec<_> = (0..64)
        .map(|p| pick_replica(&Balance::Random, 8, &[true; 8], 0, 0, p, "cache"))
        .collect();
    assert_ne!(a, b);
}
