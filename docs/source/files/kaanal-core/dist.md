# `crates/kaanal-core/src/dist.rs`

Latency distributions: how long one hop takes.

## What it does

Turns the YAML `DistConfig` (fixed, p50/p99 or mean) into a samplable `Dist`. A p50/p99 pair becomes a **lognormal** distribution fitted so its 50th and 99th percentiles land on the configured values — lognormal because real latencies have a fat right tail.

## Items

| Item                                     | Kind    | What it does                                                                                                                                                                                                                              |
| ---------------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Z99`                                    | const   | z-score of the 99th percentile of the standard normal (2.326…); used to solve for `sigma`.                                                                                                                                                |
| `Dist`                                   | enum    | `Fixed(Duration)` — always the same; `LogNormal { mu, sigma, cap_ms }` — `ln(ms) ~ N(mu, sigma)`, samples capped at `cap_ms`.                                                                                                             |
| `Dist::Exp { mean }`                     | variant | Exponential: `mean × −ln(1 − U)` with U uniform in [0, 1), capped at 10 × mean (`EXP_CAP`). `1 − U` keeps `ln(0)` impossible.                                                                                                             |
| `Dist::ZERO`                             | const   | `Fixed(0)` — the "no cost configured" value.                                                                                                                                                                                              |
| `Dist::from_p50_p99(p50, p99)`           | fn      | Fits the lognormal: `mu = ln(p50)`, `sigma = ln(p99/p50) / Z99`, cap = 10×p99 so a freak tail sample can't hang a request. Degenerate inputs (p99 ≤ p50, or either ≤ 0) fall back to `Fixed(p50)`.                                        |
| `Dist::from_config(Option<&DistConfig>)` | fn      | `None` → `ZERO`; otherwise maps the parsed YAML to a `Dist`.                                                                                                                                                                              |
| `Dist::sample(&self, rng) -> Duration`   | fn      | Draws one duration. `Fixed` returns itself; `LogNormal` samples `rand_distr::LogNormal` and clamps to the cap; `Exp` draws `mean × −ln(1 − U)` capped at 10 × mean. Takes the RNG by `&mut` so the caller controls seeding (determinism). |

## Tests

See [../tests.md](../../tests.md#crateskaanal-coretestsdistrs) — percentile fit within 5%, cap enforcement, fixed fallback, seed reproducibility.
