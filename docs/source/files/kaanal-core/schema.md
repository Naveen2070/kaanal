# `crates/kaanal-core/src/schema.rs`

The JSON Schema of `kaanal.yaml`, derived from the config types (rule R1.15).

## What it does

Builds a JSON Schema (draft 2020-12) from the `schemars::JsonSchema` derives on the types in [config.md](config.md). It is editor-only: the happy path never validates against it, serde stays the judge. Only the shapes serde cannot describe through a derive are written by hand (durations).

## Items

| Item                             | Kind  | What it does                                                                                                                                                        |
| -------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config_schema()`                | fn    | The whole schema as a `serde_json::Value`; re-exported as `kaanal_core::config_schema`.                                                                             |
| `duration_text(g)`               | fn    | **Private** (crate). Schema of a `humantime` duration string (`5ms`, `1.5s`, `2h`); used with `#[schemars(schema_with)]` on `Duration` fields (`ttl`, `token_ttl`). |
| `DURATION_PATTERN`               | const | **Private.** Loose regex for a duration with units. Only helps editors; serde has the final say.                                                                    |
| `impl JsonSchema for DistConfig` | impl  | Named `Distribution`: a `oneOf` of the three shapes (duration text, `{ p50, p99 }`, `{ mean }`), maps with `additionalProperties: false`.                           |

## Notes

Each node kind becomes one branch of `$defs.NodeConfig.oneOf`, and each branch carries `properties.type.const` (the kind's YAML name) and `additionalProperties: false`. The diagnostics added later rely on this layout; `schema_has_one_branch_per_kind` pins it.
