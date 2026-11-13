use kaanal_core::{Dist, stable_hash};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn sorted_samples_ms(d: &Dist, n: usize) -> Vec<f64> {
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let mut v: Vec<f64> = (0..n)
        .map(|_| d.sample(&mut rng).as_secs_f64() * 1000.0)
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper: sample counts and indices are small and positive"
)]
fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * q).round() as usize]
}

#[test]
fn lognormal_hits_p50_and_p99_within_5_percent() {
    let v = sorted_samples_ms(&Dist::from_p50_p99(ms(4), ms(20)), 100_000);
    let (p50, p99) = (quantile(&v, 0.50), quantile(&v, 0.99));
    assert!((p50 - 4.0).abs() / 4.0 < 0.05, "p50 = {p50}");
    assert!((p99 - 20.0).abs() / 20.0 < 0.05, "p99 = {p99}");
}

#[test]
fn fixed_is_fixed() {
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    assert_eq!(Dist::Fixed(ms(5)).sample(&mut rng), ms(5));
}

#[test]
fn p99_not_above_p50_falls_back_to_fixed() {
    assert_eq!(Dist::from_p50_p99(ms(10), ms(10)), Dist::Fixed(ms(10)));
    assert_eq!(Dist::from_p50_p99(ms(10), ms(5)), Dist::Fixed(ms(10)));
}

#[test]
fn fat_tail_is_capped_at_ten_times_p99() {
    let v = sorted_samples_ms(&Dist::from_p50_p99(ms(1), ms(1000)), 100_000);
    assert!(
        *v.last().unwrap() <= 10_000.0 + 1e-6,
        "max = {}",
        v.last().unwrap()
    );
}

#[test]
fn same_seed_same_samples() {
    let d = Dist::from_p50_p99(ms(4), ms(20));
    assert_eq!(sorted_samples_ms(&d, 100), sorted_samples_ms(&d, 100));
}

#[test]
fn stable_hash_is_fixed_and_separates_parts() {
    assert_eq!(stable_hash(&[]), 0xcbf2_9ce4_8422_2325);
    // Non-empty known answer: pins the byte loop and the separator, not just the offset.
    // The same vector is in kaanal-data's copy (generate.rs), so the two stay identical (R4.4).
    assert_eq!(stable_hash(&[b"a", b"bc"]), 0xa0a3_542c_19b9_00ab);
    assert_ne!(stable_hash(&[b"ab", b"c"]), stable_hash(&[b"a", b"bc"]));
}

#[test]
#[expect(
    clippy::cast_precision_loss,
    reason = "test: 100k samples average exactly enough in f64"
)]
fn exponential_has_the_configured_mean_and_a_cap() {
    let d = Dist::Exp { mean: ms(10) };
    let v = sorted_samples_ms(&d, 100_000);
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    assert!((mean - 10.0).abs() < 0.2, "mean {mean}");
    // Median of an exponential is mean × ln 2.
    let median = quantile(&v, 0.5);
    assert!(
        (median - 10.0 * std::f64::consts::LN_2).abs() < 0.3,
        "median {median}"
    );
    assert!(v[0] >= 0.0);
    assert!(*v.last().unwrap() <= 100.0 + 1e-9, "capped at 10 × mean");
    // Same seed → same samples.
    assert_eq!(v, sorted_samples_ms(&d, 100_000));
}

/// The mean of a lognormal is exp(mu + sigma^2 / 2) (the textbook result): the closed form is held to the
/// average of 200,000 draws (3% is far beyond the sampling error of that many, and above the sliver the
/// 10 x p99 cap trims), and an exponential's and a fixed value's mean are their own.
#[test]
#[allow(clippy::cast_precision_loss)]
fn the_mean_is_the_closed_form_and_matches_the_average_of_draws() {
    let d = Dist::from_p50_p99(ms(10), ms(50));
    // sigma = ln(5) / z99 with z99 = 2.3263: mean = 10 ms x exp(sigma^2 / 2) = 12.70 ms
    let sigma = 5f64.ln() / 2.326_347_874;
    let want = 10.0 * (sigma * sigma / 2.0).exp();
    assert!(
        (d.mean().as_secs_f64() * 1000.0 - want).abs() < 0.01,
        "{:?}",
        d.mean()
    );
    let v = sorted_samples_ms(&d, 200_000);
    let avg = v.iter().sum::<f64>() / v.len() as f64;
    assert!((avg - want).abs() / want < 0.03, "{avg} vs {want}");
    assert_eq!(Dist::Fixed(ms(7)).mean(), ms(7));
    assert_eq!(Dist::Exp { mean: ms(9) }.mean(), ms(9));
}
