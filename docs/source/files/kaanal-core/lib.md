# `crates/kaanal-core/src/lib.rs`

Crate root of **kaanal-core**: the pure logic crate (no tokio, no axum, no IO).

## What it does

- Declares the modules `chaos`, `config`, `diagnostics`, `dist`, `exec`, `presets`, `schema`, `topology`.
- Re-exports the public API so users of the crate write `kaanal_core::{Config, Topology, execute, ...}` instead of knowing module paths.
- Defines the one hash function the whole project uses for seeding.

## Items

| Item                                  | Kind | What it does                                                                                                                                                                                                                                                                                |
| ------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stable_hash(parts: &[&[u8]]) -> u64` | fn   | FNV-1a 64-bit over each part, with a `0xff` separator round between parts so `["ab","c"]` and `["a","bc"]` hash differently. Stable across platforms and Rust versions (std `Hash` is not), which is what makes seeded output reproducible. Rule R3.5: always use this, never a std hasher. |

## Re-exports

| From          | Items                                                                            |
| ------------- | -------------------------------------------------------------------------------- |
| `chaos`       | `ChaosState`, `NodeChaos`                                                        |
| `config`      | `Config`, `ConfigError`, `NodeKind`                                              |
| `diagnostics` | `Located`, `PathSeg`                                                             |
| `dist`        | `Dist`                                                                           |
| `exec`        | `ChainOutcome`, `ChainRequest`, `ChainResult`, `Hop`, `HopKind`, `Op`, `execute` |
| `topology`    | `CacheModel`, `Edge`, `Node`, `NodeId`, `Topology`, `TopologyError`              |

## Notes

`kaanal-data` keeps its own private copy of this exact algorithm (it must not depend on core, rule R2.4). The two copies are pinned identical by the test `stable_hash_matches_core_algorithm` in `kaanal-data/src/generate.rs`, which uses the same test vectors as `core/tests/dist.rs`.
