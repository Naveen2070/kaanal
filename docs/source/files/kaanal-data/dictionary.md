# `crates/kaanal-data/src/dictionary.rs`

The field dictionary: a normalised field name maps to one or more generators from a closed Rust set (design D1.2, D1.3; rules R1.49, R1.50).

## What it does

`data/dictionary.yaml` (about 150 names, compiled in) and a project's `data.yaml` share one parser, `Dictionary::parse`. A dictionary cannot run code: `Gen` is a tagged enum (`gen:` names the variant, the other keys are its parameters) with `deny_unknown_fields`, so an unknown generator or parameter is an error listing the valid ones. User guide: [docs/web/data-packs.md](../../../web/data-packs.md#writing-a-dictionary).

## Types

| Item            | Kind   | What it does                                                                                                                                                                                                                                                                                                |
| --------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Dictionary`    | struct | `exact` and `suffix` maps (key to a list of `Gen`) plus load `warnings`. Cheap to clone and share.                                                                                                                                                                                                          |
| `Gen` (private) | enum   | `first_name`, `last_name`, `full_name`, `email`, `username`, `phone`, `city`, `state`, `postcode`, `country`, `street`, `address`, `currency`, `url`, `uuid`, `integer`, `money`, `latitude`, `longitude`, `timestamp`, `date`, `text` (`kind`), `pick` (`from`), `template` (`value`), `static` (`value`). |
| `Alternatives`  | struct | **Private.** An entry: one `{ gen: ... }` or a non-empty list. Read by a visitor, not `untagged`, so errors name the wrong generator or parameter.                                                                                                                                                          |

## Functions

| Function                           | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Dictionary::parse(src)`           | Parses `version: 1` and `fields`; an empty file is an empty dictionary. Keys are normalised (`norm`: lower-case, `_` and `-` removed); a leading `*` makes a suffix key. Keys that normalise alike resolve in file order, the later wins, and each collision lands in `warnings()`. `check` validates parameters beyond their types (`min <= max`, finite bounds, template placeholders `{digits:n}` and `{letters:n}`, n 1 to 20). Errors carry line and column. |
| `Dictionary::builtin()`            | The built-in dictionary, parsed once and shared.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `lookup(field)` (private)          | The entry for a field: the exact key, else the longest suffix key that matches whole trailing words. `words()` splits at `_`, `-` and case changes (`lastOrderID` is `last`, `order`, `id`), so `*id` matches `userId` and never `paid`, and `unitprice` is one word.                                                                                                                                                                                             |
| `Gen::fits(schema, ty)` (private)  | Whether a generator can satisfy a field of scalar type `ty`: numeric generators fit `integer`/`number`, `pick` and `static` need a value of the right type inside `minimum`/`maximum`, everything else fits `string`. For a list entry the generator takes the first that fits.                                                                                                                                                                                   |
| `dictionary_warnings(model, opts)` | Walks the response schemas the way the generator does (a field with an `example`, non-empty `examples`, `const` or non-empty `enum` is not walked; only the first non-empty `oneOf`/`anyOf` is) and returns one line per project entry that matches a field but fits nowhere (`GET /users .age: project dictionary entry for age does not fit type integer or its bounds; skipped`). Built-in misfits are skipped silently (R1.50a).                              |
| `dictionary_schema()`              | The JSON Schema (draft 2020-12) for `kaanal schema dictionary`: each entry is one generator or a non-empty list of them.                                                                                                                                                                                                                                                                                                                                          |

## Rules worth knowing

- Precedence in the generator: `example`, `examples`, `const`, `enum`, `oneOf`/`anyOf`, project dictionary, `format`, built-in dictionary, constraint defaults (D1.8; see [generate.md](generate.md)).
- A project `pick`, `static` or `template` value is the developer's own data (R5.7): passed through as is, only fitted to string lengths.
- A built-in entry that does not fit is skipped silently; a project one warns (R1.50a). `kaanal validate` and `kaanal up` print `dictionary_warnings` and `Dictionary::warnings()`.

## Tests

[../../tests.md](../../tests.md#crateskaanal-datatestsdictionaryrs--the-field-dictionary): `crates/kaanal-data/tests/dictionary.rs`.
