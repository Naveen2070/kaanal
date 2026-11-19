//! The search: a ladder of load factors, then bisection of the first crossing, then the verdict.

use super::{Limit, Point, PointFail, Verdict};

/// The shape of the search.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ladder {
    /// First load factor.
    pub lowest: f64,
    /// Ratio between ladder points.
    pub step: f64,
    /// Last load factor.
    pub top: f64,
    /// Stop refining when `k_fail / k_pass` is at or under this.
    pub resolution: f64,
}

impl Default for Ladder {
    fn default() -> Self {
        Ladder {
            lowest: 0.25,
            step: std::f64::consts::SQRT_2,
            top: 8.0,
            resolution: 1.02,
        }
    }
}

impl Ladder {
    /// The ladder `k` values; `top` survives float rounding.
    pub(super) fn ks(&self) -> Vec<f64> {
        let mut ks = Vec::new();
        let mut k = self.lowest;
        while k <= self.top * (1.0 + 1e-9) {
            ks.push(k);
            k *= self.step;
        }
        ks
    }
}

/// Everything the search learned.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Searched {
    /// Ladder and refinement points, sorted by `k`.
    pub points: Vec<Point>,
    /// The finding. `HoldsTo` means held to the last point run; see `stopped_at`.
    pub verdict: Verdict,
    /// First ladder `k` above the failing stretch that passes with margin.
    pub recovers_at: Option<f64>,
    /// `k` of the point refused as too large, when the ladder was cut.
    pub stopped_at: Option<f64>,
}

/// Find where the topology first crosses `limit`.
///
/// Method: a coarse geometric ladder (`lowest * step^i` up to `top`), then bisection in log space
/// (`k = sqrt(k_pass * k_fail)`) of the first crossing until `k_fail / k_pass <= resolution`.
/// Cost: the ladder size plus `ceil(log2(ln step / ln resolution))` runs, at most 6 from a 1.414
/// step and 1.02 resolution (11 + 6 = 17 runs for the default).
///
/// # Errors
/// A ladder that cannot terminate (a field not finite, `lowest <= 0`, `step <= 1`, `lowest > top`
/// or `resolution <= 1`), a `TooLarge` at the lowest point (naming it), or any `Other` failure.
pub(crate) fn search(
    l: &Ladder,
    limit: &Limit,
    run: &mut dyn FnMut(f64) -> Result<Point, PointFail>,
) -> Result<Searched, String> {
    let finite = [l.lowest, l.step, l.top, l.resolution]
        .iter()
        .all(|x| x.is_finite());
    // `resolution` equal to 1 is refused too: bisection between adjacent floats would never stop.
    if !finite || l.lowest <= 0.0 || l.step <= 1.0 || l.lowest > l.top || l.resolution <= 1.0 {
        return Err(format!(
            "bad sweep ladder {l:?}: needs finite values, lowest > 0, step > 1, lowest <= top and resolution > 1"
        ));
    }
    let run = &mut |k: f64| {
        let r = run(k);
        if let Ok(p) = &r {
            debug_assert_eq!(
                p.k.to_bits(),
                k.to_bits(),
                "a point reports the k it was asked for"
            );
        }
        r
    };
    let ladder = l.ks();
    let mut points: Vec<Point> = Vec::new();
    let mut stopped_at = None;
    for (i, &k) in ladder.iter().enumerate() {
        match run(k) {
            Ok(p) => {
                let broken = i == 0 && !limit.passes(&p);
                points.push(p);
                if broken {
                    let p = &points[0];
                    let verdict = Verdict::BrokenAtLowest {
                        k: p.k,
                        req_s: p.req_s,
                    };
                    return Ok(Searched {
                        points,
                        verdict,
                        recovers_at: None,
                        stopped_at: None,
                    });
                }
            }
            Err(PointFail::TooLarge(m)) if i == 0 => {
                return Err(format!("the lowest point k = {k} is too large to run: {m}"));
            }
            Err(PointFail::TooLarge(_)) => {
                stopped_at = Some(k);
                break;
            }
            Err(PointFail::Other(m)) => return Err(format!("the point k = {k}: {m}")),
        }
    }

    let Some(fi) = points.iter().position(|p| !limit.passes(p)) else {
        let p = points.last().expect("the lowest point ran");
        let verdict = Verdict::HoldsTo {
            k: p.k,
            req_s: p.req_s,
        };
        return Ok(Searched {
            points,
            verdict,
            recovers_at: None,
            stopped_at,
        });
    };

    // fi >= 1: the lowest point passed (else BrokenAtLowest above). Refinement points go after
    // the ladder, so `points[..n]` stays the ladder and `pass` indexes the current best pass.
    let n = points.len();
    let mut pass = fi - 1;
    let mut fail_k = points[fi].k;
    while fail_k / points[pass].k > l.resolution {
        let mid = (points[pass].k * fail_k).sqrt();
        match run(mid) {
            Ok(p) => {
                if limit.passes(&p) {
                    pass = points.len();
                } else {
                    fail_k = p.k;
                }
                points.push(p);
            }
            Err(PointFail::TooLarge(_)) => break,
            Err(PointFail::Other(m)) => return Err(format!("the point k = {mid}: {m}")),
        }
    }

    let recovers_at = points[fi + 1..n]
        .iter()
        .find(|p| limit.passes_with_margin(p))
        .map(|p| p.k);
    let verdict = Verdict::Knee {
        k: points[pass].k,
        req_s: points[pass].req_s,
    };
    points.sort_by(|a, b| a.k.total_cmp(&b.k));
    Ok(Searched {
        points,
        verdict,
        recovers_at,
        stopped_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pt(k: f64, p99_ms: f64) -> Point {
        Point {
            k,
            req_s: 10.0 * k,
            requests: 0,
            goodput_req_s: 0.0,
            error_rate: 0.0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms,
            cause: None,
            nodes: Vec::new(),
            over: crate::sweep::Over::default(),
        }
    }

    fn lim() -> Limit {
        Limit {
            p99: Duration::from_secs(1),
            error_rate: 0.01,
        }
    }

    fn with_top(top: f64) -> Ladder {
        Ladder {
            top,
            ..Ladder::default()
        }
    }

    /// Fails (2 s) for k in (2, 4], passes at `above` ms past 4, 100 ms below.
    fn recovering(above: f64) -> impl Fn(f64) -> f64 {
        move |k| {
            if k > 4.0 {
                above
            } else if k > 2.0 {
                2000.0
            } else {
                100.0
            }
        }
    }

    #[test]
    fn knee_is_found_to_the_resolution() {
        let s = search(&with_top(16.0), &lim(), &mut |k| Ok(pt(k, 100.0 * k))).unwrap();
        let Verdict::Knee { k, req_s } = s.verdict else {
            panic!("{:?}", s.verdict)
        };
        assert!((10.0 / 1.02..=10.0).contains(&k), "knee {k}");
        assert!((req_s - 10.0 * k).abs() < 1e-9);
        for want in with_top(16.0).ks() {
            assert!(
                s.points.iter().any(|p| (p.k - want).abs() < 1e-12),
                "missing {want}"
            );
        }
        assert!(s.points.windows(2).all(|w| w[0].k <= w[1].k));
    }

    #[test]
    fn already_broken_at_the_lowest_point() {
        let mut runs = 0;
        let s = search(&Ladder::default(), &lim(), &mut |k| {
            runs += 1;
            Ok(pt(k, 2000.0))
        })
        .unwrap();
        assert!(matches!(s.verdict, Verdict::BrokenAtLowest { k, .. } if (k - 0.25).abs() < 1e-12));
        assert_eq!(runs, 1);
    }

    #[test]
    fn holds_to_the_top() {
        let s = search(&Ladder::default(), &lim(), &mut |k| Ok(pt(k, 1.0))).unwrap();
        let Verdict::HoldsTo { k, .. } = s.verdict else {
            panic!("{:?}", s.verdict)
        };
        assert!((k - 8.0).abs() < 1e-6);
        assert_eq!(s.points.len(), 11);
        assert_eq!(s.stopped_at, None);
    }

    #[test]
    fn a_recovering_curve_keeps_the_first_crossing_and_flags_recovery() {
        let f = recovering(500.0);
        let s = search(&Ladder::default(), &lim(), &mut |k| Ok(pt(k, f(k)))).unwrap();
        let Verdict::Knee { k, .. } = s.verdict else {
            panic!("{:?}", s.verdict)
        };
        assert!((2.0 / 1.02..=2.0).contains(&k), "knee {k}");
        let want = Ladder::default()
            .ks()
            .into_iter()
            .find(|&k| k > 4.0)
            .unwrap();
        assert_eq!(s.recovers_at, Some(want));
    }

    #[test]
    fn a_pass_without_margin_is_not_a_recovery() {
        let f = recovering(950.0);
        let s = search(&Ladder::default(), &lim(), &mut |k| Ok(pt(k, f(k)))).unwrap();
        assert!(matches!(s.verdict, Verdict::Knee { .. }));
        assert_eq!(s.recovers_at, None);
    }

    #[test]
    fn too_large_at_the_lowest_is_a_hard_error_naming_the_point() {
        let e = search(&Ladder::default(), &lim(), &mut |_| {
            Err(PointFail::TooLarge("big".into()))
        })
        .unwrap_err();
        assert!(e.contains("0.25"), "{e}");
    }

    #[test]
    fn too_large_above_truncates_and_never_claims_holds() {
        let mut refused = 0.0;
        let s = search(&Ladder::default(), &lim(), &mut |k| {
            if k > 2.0 + 1e-6 {
                refused = k;
                Err(PointFail::TooLarge("big".into()))
            } else {
                Ok(pt(k, 1.0))
            }
        })
        .unwrap();
        assert!(refused > 2.0);
        assert_eq!(s.stopped_at, Some(refused));
        let Verdict::HoldsTo { k, .. } = s.verdict else {
            panic!("{:?}", s.verdict)
        };
        assert!((k - 2.0).abs() < 1e-6, "held to the last point run, {k}");
    }

    #[test]
    fn refinement_runs_at_most_six_extra_points() {
        let mut runs = 0;
        let l = with_top(16.0);
        search(&l, &lim(), &mut |k| {
            runs += 1;
            Ok(pt(k, 100.0 * k))
        })
        .unwrap();
        // the ladder ran in full and refinement added at least one point, at most six
        assert!(runs > l.ks().len(), "{runs} runs, no refinement");
        assert!(runs <= l.ks().len() + 6, "{runs} runs");
    }

    #[test]
    fn a_bad_ladder_is_refused() {
        let d = Ladder::default();
        for bad in [
            Ladder { lowest: 0.0, ..d },
            Ladder { lowest: -1.0, ..d },
            Ladder { step: 1.0, ..d },
            Ladder { step: 0.5, ..d },
            Ladder { lowest: 9.0, ..d },
            Ladder {
                resolution: 0.99,
                ..d
            },
            Ladder {
                lowest: f64::NAN,
                ..d
            },
            Ladder {
                step: f64::INFINITY,
                ..d
            },
            Ladder {
                top: f64::INFINITY,
                ..d
            },
            Ladder {
                resolution: f64::NAN,
                ..d
            },
        ] {
            let mut runs = 0;
            let e = search(&bad, &lim(), &mut |k| {
                runs += 1;
                Ok(pt(k, 1.0))
            });
            assert!(e.is_err(), "{bad:?} accepted");
            assert_eq!(runs, 0, "{bad:?} ran a point");
        }
    }

    #[test]
    fn too_large_during_refinement_keeps_the_last_pass() {
        // fails past k = 10 on the ladder; every refinement point (off the ladder) is refused
        let l = with_top(16.0);
        let ladder = l.ks();
        let s = search(&l, &lim(), &mut |k| {
            if ladder.iter().any(|&x| (x - k).abs() < 1e-12) {
                Ok(pt(k, 100.0 * k))
            } else {
                Err(PointFail::TooLarge("big".into()))
            }
        })
        .unwrap();
        let Verdict::Knee { k, .. } = s.verdict else {
            panic!("{:?}", s.verdict)
        };
        assert!((k - 8.0).abs() < 1e-9, "knee {k}");
        assert_eq!(s.points.len(), ladder.len());
        assert_eq!(s.stopped_at, None);
    }

    #[test]
    fn another_error_from_a_point_ends_the_search() {
        for at in [0.25, 4.0] {
            let e = search(&Ladder::default(), &lim(), &mut |k| {
                if k >= at - 1e-9 {
                    Err(PointFail::Other(format!("boom at {k}")))
                } else {
                    Ok(pt(k, 1.0))
                }
            })
            .unwrap_err();
            assert!(
                e.starts_with("the point k = ") && e.contains("boom at"),
                "{e}"
            );
        }
    }
}
