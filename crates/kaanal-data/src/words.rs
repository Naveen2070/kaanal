//! **Private module.** Locale-free word lists for the data generator.
//!
//! Names, places and streets live in the locale packs (`data/locales/*.yaml`).

use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Lorem words for fallback strings, descriptions and URI paths.
pub const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
];

/// Uniform random element from a list, using the caller's seeded RNG.
pub fn pick<'a>(list: &[&'a str], rng: &mut ChaCha8Rng) -> &'a str {
    list[rng.random_range(0..list.len())]
}
