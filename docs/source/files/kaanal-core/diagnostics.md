# `crates/kaanal-core/src/diagnostics.rs`

Positions for config errors (rule R1.15), computed only when something is wrong.

## What it does

`serde_yaml_ng` loses positions inside the `type`-tagged `nodes:` map: an error in any node is reported at the start of `nodes:`. When parsing fails, `explain` re-reads the source with `saphyr` (a tree with line/column spans) and reports the first problem at its real place. `Located` gives errors found after parsing (graph, spec files, routes) the same `line:col`.

Diagnostics only improves the message. `Config::from_yaml` stays the sole judge: `explain` runs after serde has already failed and always returns an error. Any trouble inside it (saphyr cannot read the file, serde's position is outside `nodes:`, no node problem found, source over 1 MiB) returns serde's own error unchanged. The module denies `unwrap`, `expect`, indexing and `panic`.

## Algorithm (`explain`)

1. Skip everything if the source is over 1 MiB or has a line over 8 KiB.
2. Read the first YAML document with `saphyr`; give up if it fails.
3. If serde's line lies outside the `nodes:` entry, keep serde's error.
4. For each node, in file order: it must be a mapping with a known `type`.
5. Check the node against its own branch of the generated JSON Schema: unknown fields (listing the valid ones) and missing fields, including inside `anyOf` (optional fields).
6. Probe each field alone through serde (`{type, field}`), so bad durations, ranges, `p50 > p99` and flavors of the wrong family get the field's position and serde's own message.
7. The first problem wins; nothing found means serde's error is kept.

## Anchor rule (D7)

Errors point at the **key** of a mapping entry (node key, unknown field, spec, route key) and at the **start** of a list item (edges). Bad values point at the value, including an edge's `from`/`to` name. Missing fields point at the object. `saphyr` lines are 1-based and columns 0-based; `position` makes both 1-based.

## Items

| Item                                                          | Kind   | What it does                                                                                                     |
| ------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------- |
| `PathSeg`                                                     | enum   | One step of a path into the file: `Key(String)` or `Index(usize)`.                                               |
| `keys(parts)`                                                 | fn     | A path of mapping keys.                                                                                          |
| `Located`                                                     | struct | An error with a `path` and a `msg`, not yet a `line:col`.                                                        |
| `Located::new`                                                | fn     | An error at `path`.                                                                                              |
| `Located::in_source`                                          | fn     | Places the error in the source as a `ConfigError`; no position for an empty path or a file `saphyr` cannot read. |
| `explain(src, err)`                                           | fn     | Crate-private. Turns a serde parse error into one placed at the real node problem, or returns it unchanged.      |
| `node_error`, `check_node`, `schema_problem`, `serde_problem` | fn     | **Private.** The steps above.                                                                                    |
| `branch_for`, `allowed_fields`, `resolve`                     | fn     | **Private.** Schema lookups: a standalone branch per `type`, the valid field names, `$ref`/`anyOf` following.    |
| `tree`, `key_text`, `entry`, `locate`, `position`, `to_json`  | fn     | **Private.** `saphyr` tree helpers; `to_json` is depth-bounded.                                                  |

## Tests

[../../tests.md](../../tests.md)
