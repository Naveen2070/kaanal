# `crates/kaanal-data/src/dictionary/`

The field dictionary: a normalised field name maps to one or more generators from a closed Rust set (design D1.2, D1.3; rules R1.49, R1.50).

## Modules (R3.8)

Split from one 574-line file in D2 Task 1; public paths (`kaanal_data::Dictionary`, `dictionary_schema`, `dictionary_warnings`) are unchanged.

| File           | Holds                                                                                                       |
| -------------- | ----------------------------------------------------------------------------------------------------------- |
| `mod.rs`       | `Dictionary` (`parse`, `builtin`, `lookup`, `contains`), `norm`, `words`, the compiled-in `BUILTIN` source. |
| `generator.rs` | `Gen`, `TextKind`, `Gen::fits`, `Gen::length_fits`, `value_fits`, `template_key`.                           |
| `file.rs`      | The `data.yaml` serde types (`File`, `Key`, `Alternatives`), `dictionary_schema`, positioned errors.        |
| `check.rs`     | `check` (parameter rules), `dictionary_warnings` and its schema `walk`.                                     |

## What it does

`data/dictionary.yaml` (about 150 names, compiled in) and a project's `data.yaml` share one parser, `Dictionary::parse`. A dictionary cannot run code: `Gen` is a tagged enum (`gen:` names the variant, the other keys are its parameters) with `deny_unknown_fields`, so an unknown generator or parameter is an error listing the valid ones. User guide: [data-packs guide](https://kaanal.naveenr.in/docs/guides/mock/data-packs/).

## Types

| Item            | Kind   | What it does                                                                                                                                                                                                                                                                                                                                                                              |
| --------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Dictionary`    | struct | `exact` and `suffix` maps (key to a list of `Gen`) plus load `warnings`. Cheap to clone and share.                                                                                                                                                                                                                                                                                        |
| `Gen` (private) | enum   | `first_name`, `last_name`, `full_name`, `email`, `username`, `phone`, `city`, `state`, `postcode`, `country`, `street`, `address`, `currency`, `locale`, `language`, `url`, `uuid`, `integer` (`dist`), `money` (`dist`, `style`), `latitude`, `longitude`, `timestamp` (`days`), `date` (`days`), `text` (`kind`), `pick` (`from`, `weights`), `template` (`value`), `static` (`value`). |
| `Alternatives`  | struct | **Private.** An entry: one `{ gen: ... }` or a non-empty list. Read by a visitor, not `untagged`, so errors name the wrong generator or parameter.                                                                                                                                                                                                                                        |

## Functions

| Function                                 | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Dictionary::parse(src)`                 | Parses `version: 1` and `fields`; an empty file is an empty dictionary. Keys are normalised (`norm`: lower-case, `_` and `-` removed); a leading `*` makes a suffix key. Keys that normalise alike resolve in file order, the later wins, and each collision lands in `warnings()`. `check` validates parameters beyond their types (`min <= max`, finite bounds, template placeholders `{digits:n}` and `{letters:n}`, n 1 to 20). Errors carry line and column.                                                                                                                                                |
| `Dictionary::builtin()`                  | The built-in dictionary, parsed once and shared.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `lookup(field)` (private)                | The entry for a field: the exact key, else the longest suffix key that matches whole trailing words. `words()` splits at `_`, `-` and case changes (`lastOrderID` is `last`, `order`, `id`), so `*id` matches `userId` and never `paid`, and `unitprice` is one word.                                                                                                                                                                                                                                                                                                                                            |
| `Gen::fits(schema, ty)` (private)        | Whether a generator can satisfy a field of scalar type `ty`: numeric generators fit `integer`/`number`, `pick` and `static` need a value of the right type inside `minimum`/`maximum`, everything else fits `string`. For a list entry the generator takes the first that fits.                                                                                                                                                                                                                                                                                                                                  |
| `Gen::length_fits(schema, ty)` (private) | Whether every string a `pick` (among its type-fitting values), a `static` or a `template` can emit meets `minLength`/`maxLength` in characters; a template's length is fixed (each placeholder is exactly `n` characters). Other generators always pass (they are fitted, or well-formed, R4.5). A built-in entry must pass it to be used; a project entry that fails is used anyway and warned (R1.56).                                                                                                                                                                                                         |
| `dictionary_warnings(model, opts)`       | Walks the response schemas the way the generator does (a field with an `example`, non-empty `examples`, `const` or non-empty `enum` is not walked; only the first non-empty `oneOf`/`anyOf` is) and returns one line per project entry that matches a field but fits nowhere (`GET /users .age: project dictionary entry for age does not fit type integer or its bounds; skipped`), or whose first fitting generator fails `length_fits` (`GET /items .sku: project dictionary value for sku can fall outside its minLength/maxLength; it is sent as written`). Built-in misfits are skipped silently (R1.50a). |
| `dictionary_schema()`                    | The JSON Schema (draft 2020-12) for `kaanal schema dictionary`: each entry is one generator or a non-empty list of them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

## Rules worth knowing

- Precedence in the generator: `example`, `examples`, `const`, `enum`, `oneOf`/`anyOf`, project dictionary, `format`, built-in dictionary, constraint defaults (D1.8; see [generate.md](generate.md)).
- A `pick`, `static` or `template` string is sent as written, never trimmed or padded (R5.7, R1.56): a project one outside the field's lengths is warned, a built-in one is skipped.
- A bare `total` is a count (`integer`, 0 to 1000, whole numbers on a `number` field too); `*total` (`orderTotal`, `grandTotal`) and `subtotal`/`*subtotal` are money (R1.56).
- A built-in entry that does not fit is skipped silently; a project one warns (R1.50a). `kaanal validate` and `kaanal up` print `dictionary_warnings` and `Dictionary::warnings()`.

## Tests

[../../tests.md](../../tests.md#crateskaanal-datatestsdictionaryrs--the-field-dictionary): `crates/kaanal-data/tests/dictionary.rs`.

## Chains (D2)

`File` also reads `chains` (lists of two or more field names, earliest first) and `chains_off` (names that switch off every built-in chain containing them); both are normalised like field keys and exposed to the reconcile pass as `Dictionary::chains()` / `chains_off()`. A chain of fewer than two names is a load warning and is ignored; an empty name is a positioned error.

## Distribution keys (D2)

`Gen::Integer` gains `dist: uniform | bands`, `Gen::Money` gains `dist: decade | uniform` and `style: plain | retail`, `Gen::Timestamp` and `Gen::Date` gain `days`, and `Gen::Pick` gains `weights` (a map read by `Weights`, whose keys may be unquoted YAML scalars such as `true` or `4`, kept as their text). `check` rejects a `weights` map with no weight above 0 and `days: 0`; the list form of `weights` is a parse error. A key that matches none of a pick's values is fine: the same map weights a spec `enum`, whose values are not known at load time.

`Gen::Date` also takes `years: [min, max]` (a birth date: the age in whole years on 2026-01-01 comes from the four weighted bands, then a day inside that year; ordered, at most 150). `days` is at most 36500 (100 years) for `timestamp` and `date`. `chains_off` names that match no built-in chain member are ignored silently.
