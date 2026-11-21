# `crates/kaanal-core/src/fingerprint.rs`

Building blocks for the design fingerprint a sweep report prints, so a reader can tell whether two sweeps ran on the same design.

## What it does

`canonical_digest(text)` parses the text as YAML (JSON is YAML) into a generic value, writes it as compact JSON with sorted keys and no comments, and returns the SHA-256 (FIPS 180-4) of those bytes. Comments, whitespace and key order do not change it. Text that does not parse, or has no JSON form (for example non-string map keys), hashes its raw bytes, so a corrupt file still changes the result.

`combine(parts)` takes `(label, digest)` pairs, sorts them by label, hashes `label`, a NUL, the digest as hex and a newline for each part, and returns the first 12 lower-case hex digits. Input order does not matter; changing a digest or a label does.

`Config::from_yaml` sets `Config::digest` to `canonical_digest` of the preprocessed source it parsed. The field is `#[serde(skip)]` and `#[schemars(skip)]`: it is not part of the config schema.

## Public surface

| Item               | Kind  | Notes                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------------ | ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `canonical_digest` | fn    | `(&str) -> [u8; 32]`                                                                                                                                                                                                                                                                                                                                                                                           |
| `value_digest`     | fn    | `(&serde_json::Value) -> [u8; 32]`: the digest of a value already parsed (a loader that parsed the text hands it over instead of the text being parsed twice), streamed into SHA-256 through `serde_json::to_writer` (no buffer of the whole JSON); equal to `canonical_digest` of the text except past the 50 000 alias-node bound, where `canonical_digest` hashes raw bytes (only ever a false "different") |
| `combine`          | fn    | `(&[(String, [u8; 32])]) -> String`, 12 hex digits                                                                                                                                                                                                                                                                                                                                                             |
| `Config::digest`   | field | `[u8; 32]`, `[0; 32]` until `from_yaml` sets it                                                                                                                                                                                                                                                                                                                                                                |

Tests: `tests/fingerprint.rs`; expected hashes come from `sha256sum` on the exact bytes, not from the code.
