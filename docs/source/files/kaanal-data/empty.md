# `crates/kaanal-data/src/empty.rs`

The "no data" form of a response body, for UI-state pins (the `empty` state). Pure functions over `serde_json::Value`; the server decides when to call them ([../kaanal-server/pins.md](../kaanal-server/pins.md)). The user-facing rules are on [../../../web/pins.md](../../../web/pins.md#how-empty-is-built).

## Items

| Item                                         | Kind | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| -------------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `empty(schema, seed, opts) -> Option<Value>` | fn   | The empty form of a response described by `schema` (`$ref`s already inlined). A top-level `oneOf`/`anyOf`/`allOf` or a bare scalar has none (`None`). An array is `[]`. An object (or a schema with `properties` and no `type`) goes through the object rule. Scalars with no name rule are what `generate` gives that property with `seed` and `opts` (the owning node's pack, the built-in and project dictionaries): each property uses its own `child_seed(seed, name, 0)` with the object's seed as `group`, so they are stable per route, `oneOf` properties pick independently and adding a property changes no other value (R4.4). |
| `empty_of_value(&Value) -> Option<Value>`    | fn   | The same idea for a concrete body (an `override:` body): arrays emptied, pagination names reset, objects recursed, every other scalar kept. `None` for a bare scalar or `null`.                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

**Private:** `rule(name, num, boolean)` is the name table, matched case-insensitively and exactly: `total`, `count`, `totalCount`, `total_count`, `totalItems`, `total_items`, `totalElements`, `numFound`, `size` give `0` (number-typed properties only); `hasNext`, `has_next`, `hasMore`, `has_more` give `false` (boolean properties only); `next`, `nextCursor`, `next_cursor`, `nextPage`, `next_page` give `null` (any scalar). `object_of_schema` applies the rules per property (array to `[]`, nested object recursed with its child seed, else the name rule, else `generate::property`, sharing one value budget per body). `is_type(schema, t)` treats `type: [integer, null]` as `integer`.

## Rules worth knowing

- It is a **heuristic with a stated ceiling**: a name table, not a model. A single object with no list comes back as a generated object. The escape is the endpoint's `states.empty.body`, which the server checks first.
- A property typed as a union with `null` acts like its non-null member; an untyped property (`{}`) matches no number or boolean rule.
- The crate cannot depend on core, so this file knows nothing about pins.

## Tests

[../../tests.md](../../tests.md#cratekaanal-datatestsemptyrs--empty-bodies): `tests/empty.rs`.
