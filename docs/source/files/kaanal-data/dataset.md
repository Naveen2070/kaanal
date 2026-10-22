# `crates/kaanal-data/src/dataset.rs`

What one `generate` call draws from (`Options`) and the project's data files they come from (`DataSet`); design D1.3, D1.4, D1.10; rules R1.49 and R1.51.

## What it does

The server loads one `DataSet` at startup next to the config, then asks it for the `Options` of each route's owner node. `kaanal-data` never depends on `kaanal-core` (R2.4), so the locale arrives as a plain tag; core only validates the tag's shape and reports config errors.

## Types

| Item             | Kind   | What it does                                                                                                                                                                                                        |
| ---------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Options`        | struct | `pack` (an `Arc<Pack>`), `dictionary` (built-in, consulted after `format`) and `dictionary_project` (the project's `data.yaml`, before `format`). `Default` is the built-in `en-IN` pack and no project dictionary. |
| `DataSet`        | struct | Every pack by tag, the optional project dictionary and the load warnings.                                                                                                                                           |
| `DEFAULT_LOCALE` | const  | `en-IN`, used when neither the node nor the project sets one.                                                                                                                                                       |

## Functions

| Function                              | What it does                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DataSet::load(base_dir, data)`       | Built-in packs first, then each `locales/*.yaml` (a same-tag pack replaces a built-in), then the dictionary: `data` (relative to the config) must be a file, else `data.yaml` when it exists. A missing `locales/`, a missing `data.yaml` and an empty `data.yaml` are fine. The first problem wins, in file-name order, with `DataError::file` set; a bad `data:` has no `file` and `at = ["data"]` because it belongs to the config. |
| `pack_files(dir, warnings)` (private) | The scenario-file rules (S16): regular files named exactly `*.yaml`; subdirectories, dotfiles and `*~` skipped silently; symlinks, `.yml`/`.YAML` and non-UTF-8 names skipped with a warning; more than 64 files is an error at the directory. Packs are found by listing, a tag is never turned into a path.                                                                                                                          |
| `read(path)` (private)                | A file as UTF-8 text, refusing more than 1 MiB without reading it all (R1.30).                                                                                                                                                                                                                                                                                                                                                         |
| `options_for(tag)`                    | The `Options` of one locale. An unknown tag is an error listing the available tags, with `(did you mean <tag>?)` when only the case differs.                                                                                                                                                                                                                                                                                           |
| `tags()`, `warnings()`                | Sorted available tags; skipped files and project key collisions.                                                                                                                                                                                                                                                                                                                                                                       |
| `valid_locale_tag(s)`                 | `#[doc(hidden)]`. kaanal-data's own copy of core's tag rule `^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$`, used to vet file names. A kaanal-server test pins the two to agree.                                                                                                                                                                                                                                                                 |

## Rules worth knowing

- **Every file in `locales/` is loaded and checked**, used or not (R1.51b).
- A file name must be a valid tag and equal the pack's `tag:` exactly; two tags that differ only in case are an error naming both (R1.51c, d).
- Lookups are exact, so `../x` is just an unknown locale.

## Tests

[../../tests.md](../../tests.md#crateskaanal-datatestsdatasetrs--loading-data-files): `crates/kaanal-data/tests/dataset.rs`; the server side is `crates/kaanal-server/tests/locale.rs`.
