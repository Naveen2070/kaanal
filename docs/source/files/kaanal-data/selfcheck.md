# `crates/kaanal-data/src/selfcheck.rs`

Self-validation of generated responses (FR-DATA-05; D2 spec section 7). A generator miss is Kaanal's bug, so the findings are warnings; the repo-wide test (D2 Task 3) makes them hard failures.

## What it does

For each endpoint with a response schema it generates `SAMPLES` (5) bodies, each from `child_seed(base_seed, "selfcheck", k)`, and validates them with `jsonschema` pinned to **Draft 4** (closest to OpenAPI 3.0.x, so boolean `exclusiveMinimum`/`exclusiveMaximum` need no rewrite) with `format` checking on. A paged route is generated through `generate_page` with the window its default query resolves to, so an envelope is checked as the client sees it.

The validator gets the schema _as the generator saw it_ (`prepare`, on a copy): `writeOnly` properties leave `properties` and `required` (R6.10b); `nullable: true` becomes a type union with `null`; empty `required`/`enum`/`allOf`/`anyOf`/`oneOf` are dropped (the generator ignores them and Draft 4's meta-schema rejects them); a boolean `exclusive*` without its bound is dropped. `jsonschema` 0.58 cannot switch meta-schema validation off, so `prepare` removes what Draft 4 would reject; a schema that still fails to build yields one finding with keyword `schema`.

## Items

| Item                                          | What it does                                                                                                                                                                                                                                                                                              |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Finding`                                     | `endpoint`, `path` (JSON Pointer in the sample), `keyword`, `budget`, `sample`. `budget` is true for a `null` the body budget or depth limit cut, and for a `minItems` miss in a sample that also has one.                                                                                                |
| `SAMPLES`                                     | 5: a startup smoke check. The repo-wide test uses about 256 seeds per endpoint.                                                                                                                                                                                                                           |
| `check_endpoint(e, opts, samples, base_seed)` | All findings for one endpoint, with that endpoint's own `Options`.                                                                                                                                                                                                                                        |
| `check_model(model, opts_for, samples)`       | Warning lines for every endpoint, `opts_for` giving each endpoint's options. Per endpoint: `GET /x: 3 of 5 generated samples violate the response schema: /items/*/total: maximum` (array indexes become `*`, three kinds shown, then `(and N more)`), and a separate "body budget" line for budget cuts. |

## Tests

[../../tests.md](../../tests.md#crateskaanal-datatestsselfcheckrs--self-validation-d2): `crates/kaanal-data/tests/selfcheck.rs`.

## Review fixes

On a paged route `minItems` is dropped from the paged array in the validator's copy (a page holds at most one page size, however large `minItems` is). A pattern with its own startup warning (unsupported, or no candidate fits its length bounds) is not reported again; the walk looks at schema positions only, never `example` or `enum` values. The ignored-keyword list is `warn::IGNORED` (one list). Values that are `null` are reported as "the body budget or depth limit turned values into null" (a user `example: null` is the same line; the advice is a smaller `minItems` or a shallower schema).
