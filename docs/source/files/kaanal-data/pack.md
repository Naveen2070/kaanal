# `crates/kaanal-data/src/pack.rs`

Locale packs: the names, places, phone shape, currency and country the generator draws from (design D1.5, D1.6; rules R1.49 and R1.51).

## What it does

A pack is one YAML file. The built-in `en-IN` and `en-US` packs (`crates/kaanal-data/data/locales/`) are compiled in with `include_str!` and load through the same `Pack::parse` as a project's `locales/<tag>.yaml`, so there is one parser and one set of checks. Users write packs with [docs/web/data-packs.md](../../../web/data-packs.md).

## Types

| Item                         | Kind   | What it does                                                                                                                                                                                                 |
| ---------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Pack`                       | struct | The file: `version`, `tag`, `name_order`, `country`, `currency`, `phone`, `address`, `first_names`, `last_names`, `streets`, `places`. `deny_unknown_fields`; the lists must not be empty.                   |
| `NameOrder`                  | enum   | `given-family` or `family-given`, for `full_name`.                                                                                                                                                           |
| `Currency`, `Phone`, `Place` | struct | `{ code, symbol }`; `{ prefix, formats }`; `{ city, state?, postcode }`. `{n}` in a phone format or postcode is n seeded digits.                                                                             |
| `Name`                       | struct | `name` as written plus an optional ASCII `latin`. In YAML a plain string or `{ name, latin }`, read by a hand-written visitor (`untagged` would lose the position).                                          |
| `DataError`                  | struct | A pack or data-file error: `line`/`col` when serde knows them, else `at` (a key path such as `["places", "3"]`) with `value` saying whether to point at the value (R1.30). `file` is set by `DataSet::load`. |

## Functions

| Function                                          | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Pack::parse(src)`                                | Strips a BOM, deserialises, then checks across fields: `version` is 1, the `address` template uses only `{number}`, `{street}`, `{city}`, `{state}`, `{postcode}` and has no unclosed brace, phone formats and postcodes use only `{n}` (1 to 20), and an address with `{state}` needs a state on every place (the error names the place). A `latin:` that is not ASCII, or an empty name, fails inside the visitor so the error carries the scalar's own position. |
| `Pack::builtin(tag)`                              | A built-in pack by exact tag, parsed once and shared (`Arc`). A broken built-in panics at first use, which the tests hit.                                                                                                                                                                                                                                                                                                                                           |
| `builtin_tags()`                                  | Tags of the compiled-in packs. The private `BUILTIN` list is the one place to extend when a pack is added.                                                                                                                                                                                                                                                                                                                                                          |
| `Pack::ascii_local(name, n)`                      | The lower-case ASCII local part for emails and usernames: the `latin:` form, else the name with Latin-1 and Latin Extended-A letters folded (`José` to `jose`, `Straße` to `strasse`), else `user<n>`. Only letters and digits survive. Never panics on any Unicode input.                                                                                                                                                                                          |
| `pack_schema()`                                   | The pack's JSON Schema (draft 2020-12, from `schemars`) for `kaanal schema pack`. Private helper types carry explicit descriptions so no internal doc text leaks into it.                                                                                                                                                                                                                                                                                           |
| `fill`, `digit_count`, `non_empty`, `serde_error` | `pub(crate)` helpers: placeholder expansion, `{n}` parsing, the non-empty-list visitor and a serde error without its position repeated in the message. Shared with [dictionary.rs](dictionary.md).                                                                                                                                                                                                                                                                  |

## Rules worth knowing

- A project pack replaces a built-in of the same tag wholesale; there is no merge (the dictionary is the merge surface).
- `{number}` in an address is a seeded 1 to 999; a `state` generator on a stateless place returns the city.
- Pack files are found, named and limited by [dataset.rs](dataset.md); this file only parses one.
- The built-in files are marked synthetic in a header comment (FR-09, R5.7).

## Tests

[../../tests.md](../../tests.md#crateskaanal-datatestspackrs--locale-packs): `crates/kaanal-data/tests/pack.rs`.
