# `crates/kaanal-core/src/units.rs`

The one parser for byte and rate values in the config (F7, RULES 55).

## What it does

A byte field is a whole number (bytes; a rate: bytes a second, as it always was) or text with a unit: `250KB`, `250 KB`, `1.5MB`, `512B`, and for a rate `1MB/s`. Units are `B`, `KB`, `MB`, `GB`, any case, with `KiB`, `MiB`, `GiB` the same thing; **KB = 1024 bytes**, the same as Studio's unit picker. A fraction rounds to the nearest byte; whole numbers take an exact `u64` path. A unit-less string counts as bytes. The bounds (a rate at least 1, a size at most 1 GiB) stay in each field's own check (`config/checks.rs`, `config/traffic.rs`, `chaos`) and run on the converted number.

`parse(text, rate)` returns the bytes or a message without a field name. `field(d, name, rate)` is the shared body of the `deserialize_with` functions `bandwidth`, `out`, `inbound` (the `in` key), `bandwidth_cap`, `req_bytes` and `resp_bytes`; each puts its field name in front (for example "bandwidth: unknown unit in 5XB; use B, KB, MB, GB, optionally /s for a rate") and reads an absent or null value as `None`. The value is read through a visitor, so YAML (`serde_yaml_ng`) and the control API's JSON (`serde_json`) give the same result and text. A YAML float (`1.5`) or a negative number is refused with the same kind of message; write a fraction as text with a unit.

`NodeBandwidth` (the one-number form of a node's `bandwidth`) has its own `Deserialize` that calls `node_rate` for text.

## Errors

| Text                      | Message                                                                                 |
| ------------------------- | --------------------------------------------------------------------------------------- |
| no number (`KB`, `NaNKB`) | `` `KB` has no number; write a number and a unit (B, KB, MB, GB) ``                     |
| `5.KB`                    | `` `5.KB` has no digits after the point; ... ``                                         |
| negative                  | `` `-5KB` is negative; use a number of 0 or more ``                                     |
| unknown unit              | ``unknown unit in `5XB`; use B, KB, MB, GB`` (a rate adds `, optionally /s for a rate`) |
| `/s` on a size            | `` `5KB/s` is a size, not a rate; drop the `/s` (units: B, KB, MB, GB) ``               |
| past a `u64`              | `` `...` is too large (at most 18446744073709551615 bytes) ``                           |

## Public surface

| Item                                                                      | Kind  | Notes                                                                                |
| ------------------------------------------------------------------------- | ----- | ------------------------------------------------------------------------------------ |
| `parse(text, rate)`                                                       | fn    | Text to bytes. Used by `kaanal chaos set --bandwidth-cap`.                           |
| `node_rate(text)`                                                         | fn    | Text to a node's rate with the `bandwidth:` prefix, for the `NodeBandwidth` visitor. |
| `bandwidth`, `out`, `inbound`, `bandwidth_cap`, `req_bytes`, `resp_bytes` | fn    | `deserialize_with` functions returning `Option<u64>`.                                |
| `Raw`, `RawVisitor`, `field`                                              | mixed | **Private.** What a value was before parsing, its visitor and the shared body.       |
