# `crates/kaanal-data/src/words.rs`

Locale-free word list for the data generator. Private to the crate.

## What it does

Names, places and streets used to live here (one hardcoded `en-IN` pack). They now live in the locale packs (`crates/kaanal-data/data/locales/*.yaml`, see [pack.md](pack.md)). What stays is the one list that has no locale.

## Items

| Item              | Kind  | Contents and use                                                                                            |
| ----------------- | ----- | ----------------------------------------------------------------------------------------------------------- |
| `WORDS`           | const | 20 lorem words: fallback strings, `text` sentences and `uri`/`hostname` formats in [generate](generate.md). |
| `pick(list, rng)` | fn    | Uniform random element from a list using the caller's seeded RNG.                                           |
