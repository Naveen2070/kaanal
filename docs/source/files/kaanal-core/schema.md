# `crates/kaanal-core/src/schema.rs`

The JSON Schema of `kaanal.yaml`, derived from the config types (rule R1.15).

## What it does

Builds a JSON Schema (draft 2020-12) from the `schemars::JsonSchema` derives on the types in [config.md](config.md). It is editor-only: the happy path never validates against it, serde stays the judge. Only the shapes serde cannot describe through a derive are written by hand (durations).

## Items

| Item                                  | Kind  | What it does                                                                                                                                                                                                                                                                                |
| ------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config_schema()`                     | fn    | The whole schema as a `serde_json::Value`; re-exported as `kaanal_core::config_schema`.                                                                                                                                                                                                     |
| `rate(g)`, `rate_value(g)`, `size(g)` | fn    | **Private** (crate). Schema of a byte rate or size field: an integer (rate at least 1) or a string matching `RATE_PATTERN` / `SIZE_PATTERN` (`250KB`, `1.5MB/s`), plus `null` for an optional field; `schema_with` on `bandwidth`, `out`, `in`, `bandwidth_cap`, `req_bytes`, `resp_bytes`. |
| `duration_text(g)`                    | fn    | **Private** (crate). Schema of a `humantime` duration string (`5ms`, `1.5s`, `2h`); used with `#[schemars(schema_with)]` on `Duration` fields (`ttl`, `token_ttl`).                                                                                                                         |
| `claims(g)`                           | fn    | **Private** (crate). Schema of a `claims` map: an object of at most 32 string, number or boolean values; used with `#[schemars(schema_with)]` on `AuthConfig.claims` and `UserConfig.claims`. Load rules are in [config.md](config.md).                                                     |
| `pins(g)`                             | fn    | **Private** (crate). Schema of a scenario's `pins`: the list of `PinEntryConfig`, or null (read as empty).                                                                                                                                                                                  |
| `DURATION_PATTERN`                    | const | **Private.** Loose regex for a duration with units. Only helps editors; serde has the final say.                                                                                                                                                                                            |
| `impl JsonSchema for DistConfig`      | impl  | Named `Distribution`: a `oneOf` of the three shapes (duration text, `{ p50, p99 }`, `{ mean }`), maps with `additionalProperties: false`.                                                                                                                                                   |
| `impl JsonSchema for StepConfig`      | impl  | Named `Step`: a `oneOf` of a node name, `call` (with `op`, `repeat`, `chain`, `with`, `via`), `cache` and `parallel` (2 to 32 branches). Recursive through `$ref`: the schema generator resolves the self-reference by name.                                                                |
| `impl JsonSchema for ErrorMapping`    | impl  | Named `ErrorMapping`: a status (200 to 599) or `{ status, body }`.                                                                                                                                                                                                                          |
| `impl JsonSchema for EndpointsConfig` | impl  | Named `Endpoints`: a path string or a map of `EndpointConfig` entries.                                                                                                                                                                                                                      |

## Notes

Each node kind becomes one branch of `$defs.NodeConfig.oneOf`, and each branch carries `properties.type.const` (the kind's YAML name) and `additionalProperties: false`. The diagnostics added later rely on this layout; `schema_has_one_branch_per_kind` pins it.

## Committed copy

`crates/kaanal-core/kaanal.schema.json`, checked by `committed_schema_matches_the_types` as JSON values (line endings do not matter). B2a re-blessed it for the new `timeout` field on every node kind (and the `error_map` key notes). Regenerate with `KAANAL_BLESS=1 cargo test -p kaanal-core --test schema`, then review the diff. Every field needs a doc comment (it becomes the description); inert fields say "no effect yet". `concurrency` has a helper, `schema::concurrency`: an integer of at least 1, or the `BoundedConcurrency` map. `kaanal schema` prints the same document. Publishing URL default: `https://kaanal.naveenr.in/schema/v1.json` (not published in A1).
