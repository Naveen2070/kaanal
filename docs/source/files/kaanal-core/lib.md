# `crates/kaanal-core/src/lib.rs`

Crate root of **kaanal-core**: the pure logic crate (no tokio, no axum, no IO).

## What it does

- Declares the modules `chain`, `chaos`, `config`, `diagnostics`, `dist`, `exec`, `pin`, `presets`, `replay`, `scenario`, `schema`, `topology`.
- Re-exports the public API so users of the crate write `kaanal_core::{Config, Topology, execute, ...}` instead of knowing module paths.
- Defines the one hash function the whole project uses for seeding.

## Items

| Item                                  | Kind | What it does                                                                                                                                                                                                                                                                                |
| ------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stable_hash(parts: &[&[u8]]) -> u64` | fn   | FNV-1a 64-bit over each part, with a `0xff` separator round between parts so `["ab","c"]` and `["a","bc"]` hash differently. Stable across platforms and Rust versions (std `Hash` is not), which is what makes seeded output reproducible. Rule R3.5: always use this, never a std hasher. |

## Re-exports

| From          | Items                                                                                                                                                                                                          |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `chain`       | `Endpoint`, `Endpoints`, `EndpointsInput`, `JWKS_KEY`, `LOGIN_KEY`, `Responder`, `RouteSpec`, `compile`, `resolve_error`, `skeleton`                                                                           |
| `chaos`       | `ChaosKey`, `ChaosLayer`, `ChaosState`, `EffectiveChaos`, `ErrorKind`, `FailKind`, `Fault`, `HANG_CAP`, `MAX_FAULTS`, `MAX_LATENCY_ADD`, `NodeChaos`, `Resolved`, `compile_layers`                             |
| `config`      | `ChaosEntry`, `Config`, `ConfigError`, `Determinism`, `EndpointsConfig`, `NodeKind`, `ScenarioConfig`, `SeedSource`, `parse_scenario`                                                                          |
| `diagnostics` | `Located`, `PathSeg`                                                                                                                                                                                           |
| `dist`        | `Dist`                                                                                                                                                                                                         |
| `exec`        | `ChainOutcome`, `ChainRequest`, `ChainResult`, `Hop`, `HopKind`, `Op`, `Plan`, `PlanItem`, `execute`, `sample_plan`                                                                                            |
| `pin`         | `EffectivePins`, `EmptyOk`, `Outcome`, `PinEntry`, `PinEntryConfig`, `PinLayer`, `PinSource`, `PinState`, `ResolvedPin`, `StateOverride`, `StateOverrideConfig`, `StateOverrides`, `resolve`, `validate_entry` |
| `replay`      | `Engine`, `EngineError`, `Finished`, `FinishedHop`, `NodeCounters`                                                                                                                                             |
| `scenario`    | `ChaosStack`, `MAX_ENTRIES`, `MAX_SCENARIOS`, `Scenario`, `ScenarioError`, `ScenarioSet`, `Source`, `endpoint_key`, `resolve_scenario`, `valid_name`                                                           |
| `topology`    | `Admission`, `CacheModel`, `Edge`, `Node`, `NodeId`, `RateLimit`, `Topology`, `TopologyError`                                                                                                                  |

## Notes

`kaanal-data` keeps its own private copy of this exact algorithm (it must not depend on core, rule R2.4). The two copies are pinned identical by the test `stable_hash_matches_core_algorithm` in `kaanal-data/src/generate.rs`, which uses the same test vectors as `core/tests/dist.rs`.
