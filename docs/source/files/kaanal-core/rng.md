# `crates/kaanal-core/src/rng.rs`

The random generator behind every hop's draws.

## What it does

`HopRng` is SplitMix64: a 64-bit state, one add and a three-step mix per draw, with `RngCore` and `SeedableRng` implemented so `rand::Rng` and `rand_distr` work on it. `seed_from_u64(s)` stores `s` as the state, so seeding costs one store.

Every hop seeds its own generator from `stable_hash(path, node, tag)` (R1.34) and draws one to three values, so seeding was the cost: `ChaCha8Rng` spent most of a request on key expansion and a 256-byte block per hop. Switching cut `sample_plan` by about 36% and `execute` by about 39%.

## Limits

Not for cryptography. Two hops overlap only if their 64-bit hashes land within a few draws of each other, which FNV-1a over distinct (path, node, tag) does not do in practice. The simulator's arrival streams and `kaanal-data` still use `ChaCha8Rng`.

## Public surface

| Item     | Kind   | Notes                                             |
| -------- | ------ | ------------------------------------------------- |
| `HopRng` | struct | `Clone`, `Debug`; re-exported from `kaanal_core`. |
