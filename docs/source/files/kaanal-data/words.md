# `crates/kaanal-data/src/words.rs`

Word lists for the data generator's heuristics. Private to the crate.

## What it does

Synthetic-only source data (rule R5.7: no real-person datasets). Currently one locale pack, `en-IN`.

## Items

| Const     | Contents                                | Used for                                                                                                                        |
| --------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `FIRST`   | 20 Indian first names (Aarav, Priya, …) | `firstName`, `name`, email local parts — one person per object via `person()` in [generate](generate.md)                        |
| `LAST`    | 20 Indian surnames (Sharma, Iyer, …)    | `lastName`, `name`, emails                                                                                                      |
| `CITIES`  | 10 `(city, state, pincode)` triples     | `city`/`state`/`pincode`/`address` — always consistent because `place()` in [generate](generate.md) picks one triple per object |
| `STREETS` | 8 street names                          | `address`                                                                                                                       |
| `WORDS`   | 20 lorem words                          | fallback strings, descriptions, URI paths                                                                                       |

| Function          | What it does                                                      |
| ----------------- | ----------------------------------------------------------------- |
| `pick(list, rng)` | Uniform random element from a list using the caller's seeded RNG. |

## Adding a locale pack (future, FR-DATA-03)

Add parallel lists (e.g. `FIRST_US`) and thread a locale choice through `generate`'s heuristics; keep `pick` generic. Determinism rule: the locale must come from config, never from the environment.
