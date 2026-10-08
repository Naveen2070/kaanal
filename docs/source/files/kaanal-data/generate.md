# `crates/kaanal-data/src/generate.rs`

The data generator: deterministic, realistic JSON from an OpenAPI 3.0 schema.

## What it does

`generate(schema, field, seed)` produces a `serde_json::Value` that validates against the schema and looks like production data (real names, emails, pincodes — not `"string"` / `0`). Same seed → same output, always.

Field-pick priority (first match wins): **`example` → `enum` → `allOf` (merge) / `oneOf`/`anyOf` (first) → type**, and within a type: **`format` → field-name heuristic → constraints → fallback**.

## Public functions

| Function                                 | What it does                                                                                                                                                                                                      |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `generate(schema, field, seed) -> Value` | Entry point. `field` is the property name this value is for (drives heuristics).                                                                                                                                  |
| `apply_params(body, params)`             | Copies URL path params into same-named top-level fields when types fit (`/users/42` → `id: 42`). Strings overwrite strings; numeric strings overwrite numbers; a non-numeric param never corrupts a number field. |

## Seeding model (rule R4.4)

- `stable_hash` — a private copy of core's FNV-1a (this crate cannot depend on core; the test `stable_hash_matches_core_algorithm` pins them identical).
- `child_seed(parent, name, index)` — every child value derives its own seed. Adding, removing or reordering schema properties never changes the values of other fields (tested).
- `group` — the _enclosing object's_ seed. Fields that must agree with each other (city/state/pincode/address) derive their shared choice from `group`, not from their own seeds, so one object always describes one place.

## Key internal functions

| Function                                           | What it does                                                                                                                                                                                                                                                                                                                        |
| -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `value_for(s, field, seed, group, depth)`          | The recursive dispatcher implementing the priority order above. Depth-capped at 16.                                                                                                                                                                                                                                                 |
| `is_id(field)`                                     | `id`, `userId`, `ID`, `user_id`, `-id` → true; `valid`, `paid`, `android` → false.                                                                                                                                                                                                                                                  |
| `norm(field)`                                      | `firstName` / `first_name` / `first-name` → `firstname` for heuristic lookup.                                                                                                                                                                                                                                                       |
| `int_for(s, field, rng)`                           | Heuristic ranges: `age` 18–80, `quantity/qty/count` 1–10, ids 1–99 999, default 0–1000. `minimum`/`maximum` (and OpenAPI 3.0 boolean `exclusiveMinimum`/`exclusiveMaximum`) override the range.                                                                                                                                     |
| `number_for(s, field, rng)`                        | `lat`/`latitude` 8–35, `lng`/`lon`/`longitude` 68–97 (India); money-suffixed fields (`price`, `total`, `amount`, `cost`, `subtotal`) 99–4999 rounded to 2 decimals; bounds applied with one-cent slack for exclusive limits.                                                                                                        |
| `string_for(s, field, rng, group)`                 | Dispatches on `format` (email, uuid, date-time, date, uri, ipv4) then `heuristic()`, then a lorem word. **Formats beat lengths** (rule R4.5): `fit_length` is skipped for formatted or well-formed heuristic values — a truncated email is worse than a violated `maxLength`.                                                       |
| `heuristic(f, id_like, group, rng)`                | Field-name dictionary: names and emails (via `person(group)`, so `firstName`/`lastName`/`name`/`email` agree), `+91` phones, city/state/pincode/address (via `place(group)`), country/currency (India/INR), timestamps, descriptions, `status: active`, `*url` fields, id-like strings → uuid. Returns `None` when nothing matches. |
| `email`, `uuid`, `digits`, `sentence`, `date_time` | Value builders. `uuid` sets the v4 variant/version bits; `date_time` is always within the 365 days before 2026-01-01 (`BASE_EPOCH`, fixed so output never drifts between runs); emails use `example.com` only (rule R5.7).                                                                                                          |
| `civil_from_days(z)`                               | Days since epoch → (year, month, day); Howard Hinnant's algorithm.                                                                                                                                                                                                                                                                  |
| `fit_length(v, s)`                                 | Applies `maxLength` (truncate) and `minLength` (pad with `x`) to plain strings only.                                                                                                                                                                                                                                                |

## Limits

- `MAX_DEPTH = 16` — pathological nesting returns `null` instead of recursing forever.
- `MAX_RANDOM_ITEMS = 20` — `maxItems: 1000000` cannot build a huge array (`minItems` is still honoured).

## Unit tests in this file

`stable_hash_matches_core_algorithm` (same vectors as core) and `civil_dates` (epoch, `BASE_EPOCH` = 2026-01-01, leap day 2024-02-29). Integration tests: [../tests.md](../../tests.md#crateskaanal-datatestsgeneraters).
