# `crates/kaanal-data/src/lib.rs`

Crate root of **kaanal-data**: OpenAPI parsing and deterministic data generation. Never depends on `kaanal-core` (rule R2.4).

## What it does

Declares private modules and re-exports the whole public surface:

| Re-export                  | From       | What it is                                                            |
| -------------------------- | ---------- | --------------------------------------------------------------------- |
| `ApiModel`, `Endpoint`     | `api`      | Parsed OpenAPI 3.0 spec → endpoint list. See [api.md](api.md).        |
| `generate`, `apply_params` | `generate` | Deterministic mock JSON for a schema. See [generate.md](generate.md). |
| `empty`, `empty_of_value`  | `empty`    | The "no data" form of a response body. See [empty.md](empty.md).      |

`words` (name/city/word lists) stays private — see [words.md](words.md).
