# `crates/kaanal-data/src/lib.rs`

Crate root of **kaanal-data**: OpenAPI parsing, data files (locale packs and the field dictionary) and deterministic data generation. Never depends on `kaanal-core` (rule R2.4).

## What it does

Declares private modules and re-exports the public surface:

| Re-export                                                                                             | From         | What it is                                                                                      |
| ----------------------------------------------------------------------------------------------------- | ------------ | ----------------------------------------------------------------------------------------------- |
| `ApiModel`, `Endpoint`                                                                                | `api`        | Parsed OpenAPI 3.0 to endpoint list. See [api.md](api.md).                                      |
| `generate`, `apply_params`                                                                            | `generate`   | Deterministic mock JSON for a schema. See [generate.md](generate.md).                           |
| `empty`, `empty_of_value`                                                                             | `empty`      | The "no data" form of a response body. See [empty.md](empty.md).                                |
| `DataSet`, `Options`, `DEFAULT_LOCALE`, `valid_locale_tag` (hidden)                                   | `dataset`    | The project's data files and what one `generate` call draws from. See [dataset.md](dataset.md). |
| `Pack`, `Name`, `NameOrder`, `Currency`, `Phone`, `Place`, `DataError`, `builtin_tags`, `pack_schema` | `pack`       | Locale packs and the data-file error. See [pack.md](pack.md).                                   |
| `Dictionary`, `dictionary_schema`, `dictionary_warnings`                                              | `dictionary` | The field dictionary. See [dictionary.md](dictionary.md).                                       |

`warn` (ignored-keyword and discriminator warnings, see [warn.md](warn.md)) is private and reached through `ApiModel::warnings()`. `words` (the lorem word list) stays private, see [words.md](words.md).
