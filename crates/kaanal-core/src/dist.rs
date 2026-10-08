//! Latency distributions: how long one hop takes.
//!
//! A configured p50/p99 pair becomes a lognormal (real latencies have a fat right
//! tail); a plain duration is fixed. Sampling always goes through a caller-owned
//! RNG so outcomes stay deterministic.

use std::time::Duration;

use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, LogNormal};

use crate::config::DistConfig;

/// **Private.** z-score of the 99th percentile of the standard normal distribution.
const Z99: f64 = 2.326_347_874;

/// A samplable latency distribution.
#[derive(Debug, Clone, PartialEq)]
pub enum Dist {
    /// Always the same duration.
    Fixed(Duration),
    /// Lognormal over milliseconds: ln(ms) ~ N(mu, sigma). Samples are capped at `cap_ms`.
    LogNormal {
        /// Location parameter: `ln(p50_ms)`.
        mu: f64,
        /// Shape parameter: `ln(p99_ms / p50_ms) / Z99`.
        sigma: f64,
        /// Hard ceiling per sample (10×p99), so a fat tail cannot hang a request.
        cap_ms: f64,
    },
}

impl Dist {
    /// Zero duration — the "no cost configured" value.
    pub const ZERO: Dist = Dist::Fixed(Duration::ZERO);

    /// Fits a lognormal to the given percentiles.
    ///
    /// Degenerate inputs (either ≤ 0, or p99 ≤ p50) fall back to `Fixed(p50)`.
    pub fn from_p50_p99(p50: Duration, p99: Duration) -> Dist {
        let (a, b) = (p50.as_secs_f64() * 1000.0, p99.as_secs_f64() * 1000.0);
        if a <= 0.0 || b <= a {
            return Dist::Fixed(p50);
        }
        // limitation: cap at 10×p99 so a fat tail can't hang a request; make it configurable if anyone needs longer
        Dist::LogNormal {
            mu: a.ln(),
            sigma: (b / a).ln() / Z99,
            cap_ms: b * 10.0,
        }
    }

    /// Maps a parsed YAML distribution; `None` means "no cost" → [`Dist::ZERO`].
    pub fn from_config(c: Option<&DistConfig>) -> Dist {
        match c {
            None => Dist::ZERO,
            Some(DistConfig::Fixed(d)) => Dist::Fixed(*d),
            Some(DistConfig::Pct { p50, p99 }) => Dist::from_p50_p99(*p50, *p99),
        }
    }

    /// Draws one duration from the distribution.
    ///
    /// The RNG is caller-owned: who seeds it controls the outcome (determinism rule R4.1).
    pub fn sample(&self, rng: &mut ChaCha8Rng) -> Duration {
        match self {
            Dist::Fixed(d) => *d,
            Dist::LogNormal { mu, sigma, cap_ms } => {
                let ms: f64 = LogNormal::new(*mu, *sigma).expect("sigma > 0").sample(rng);
                Duration::from_secs_f64(ms.min(*cap_ms) / 1000.0)
            }
        }
    }
}
