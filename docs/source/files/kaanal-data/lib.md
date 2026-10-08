# `crates/kaanal-data/src/lib.rs`

Crate root of **kaanal-data**: OpenAPI parsing and deterministic data generation. Never depends on `kaanal-core` (rule R2.4).

## What it does

Declares private modules and re-exports the whole public surface:

| Re-export | From | What it is |
|---|---|---|
| `ApiModel`, `Endpoint`, `ApiError` | `api` | Parsed OpenAPI 3.0 spec → endpoint list. See [data-api.md](api.md). |
| `generate`, `apply_params` | `generate` | Deterministic mock JSON for a schema. See [data-generate.md](generate.md). |
| `unsupported_keywords` | `warn` | Warnings for schema keywords the generator ignores. See [data-warn.md](warn.md). |

`words` (name/city/word lists) stays private — see [data-words.md](words.md).
