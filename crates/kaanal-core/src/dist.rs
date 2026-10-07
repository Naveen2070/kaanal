use std::time::Duration;

use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, LogNormal};

use crate::config::DistConfig;

/// z-score of the 99th percentile of the standard normal distribution.
const Z99: f64 = 2.326_347_874;

#[derive(Debug, Clone, PartialEq)]
pub enum Dist {
    Fixed(Duration),
    /// Lognormal over milliseconds: ln(ms) ~ N(mu, sigma). Samples are capped at `cap_ms`.
    LogNormal {
        mu: f64,
        sigma: f64,
        cap_ms: f64,
    },
}

impl Dist {
    pub const ZERO: Dist = Dist::Fixed(Duration::ZERO);

    pub fn from_p50_p99(p50: Duration, p99: Duration) -> Dist {
        let (a, b) = (p50.as_secs_f64() * 1000.0, p99.as_secs_f64() * 1000.0);
        if a <= 0.0 || b <= a {
            return Dist::Fixed(p50);
        }
        // ponytail: cap at 10×p99 so a fat tail can't hang a request; make it configurable if anyone needs longer
        Dist::LogNormal {
            mu: a.ln(),
            sigma: (b / a).ln() / Z99,
            cap_ms: b * 10.0,
        }
    }

    pub fn from_config(c: Option<&DistConfig>) -> Dist {
        match c {
            None => Dist::ZERO,
            Some(DistConfig::Fixed(d)) => Dist::Fixed(*d),
            Some(DistConfig::Pct { p50, p99 }) => Dist::from_p50_p99(*p50, *p99),
        }
    }

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
