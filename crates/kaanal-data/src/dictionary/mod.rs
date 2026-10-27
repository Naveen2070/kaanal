//! The field dictionary (design D1.2, D1.3): a normalised field name maps to one or more
//! generators from a closed Rust set. The built-in dictionary is YAML compiled in and
//! loads through the same [`Dictionary::parse`] as a project's `data.yaml`.

mod check;
mod file;
mod generator;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::pack::{DataError, serde_error};
use file::File;

pub use check::dictionary_warnings;
pub use file::dictionary_schema;
pub(crate) use generator::{
    Gen, IntDist, MoneyDist, MoneyStyle, TextKind, template_key, value_fits,
};

/// A field dictionary: exact keys and `*suffix` keys, both normalised.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    exact: HashMap<String, Vec<Gen>>,
    suffix: HashMap<String, Vec<Gen>>,
    warnings: Vec<String>,
    chains: Vec<Vec<String>>,
    chains_off: Vec<String>,
}

/// **Private.** `firstName`, `first_name`, `First-Name` → `firstname`.
pub(crate) fn norm(field: &str) -> String {
    field
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// **Private.** Lower-case words of a field name, split at `_`, `-` and case changes:
/// `lastOrderID` → `last`, `order`, `id`; `IDCard` → `id`, `card`.
fn words(field: &str) -> Vec<String> {
    let cs: Vec<char> = field.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c == '_' || c == '-' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            let prev = cs[i - 1];
            let next_lower = cs.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// **Private.** The built-in dictionary source.
const BUILTIN: &str = include_str!("../../data/dictionary.yaml");

impl Dictionary {
    /// Parses and checks a dictionary file (`version`, `fields`). An empty file is an
    /// empty dictionary. Keys are normalised; a key starting `*` matches a suffix of
    /// whole words (`*id` matches `userId`, not `paid`).
    ///
    /// # Errors
    /// A [`DataError`] with line and column: unknown generator or parameter (listing the
    /// valid ones), bad template, range or bound, empty key, unsupported version.
    pub fn parse(src: &str) -> Result<Dictionary, DataError> {
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let file: Option<File> = serde_yaml_ng::from_str(src).map_err(|e| serde_error(&e))?;
        let Some(file) = file else {
            return Ok(Dictionary::default());
        };
        let mut d = Dictionary::default();
        // (suffix?, normalised) → the original key that set it, to report collisions
        let mut seen: HashMap<(bool, String), String> = HashMap::new();
        for (key, gens) in file.fields {
            let (is_suffix, k) = match key.strip_prefix('*') {
                Some(rest) => (true, norm(rest)),
                None => (false, norm(&key)),
            };
            if let Some(prev) = seen.insert((is_suffix, k.clone()), key.clone()) {
                d.warnings.push(format!(
                    "fields: `{prev}` and `{key}` are the same key after normalising; the later `{key}` wins"
                ));
            }
            let map = if is_suffix {
                &mut d.suffix
            } else {
                &mut d.exact
            };
            map.insert(k, gens);
        }
        for chain in file.chains {
            let names: Vec<String> = chain.iter().map(|k| norm(&k.0)).collect();
            if names.len() < 2 {
                d.warnings
                    .push("chains: a chain needs at least two names; ignored".to_string());
            } else {
                d.chains.push(names);
            }
        }
        d.chains_off = file.chains_off.iter().map(|k| norm(&k.0)).collect();
        Ok(d)
    }

    /// **Private.** Project chains (normalised names, earliest first).
    pub(crate) fn chains(&self) -> &[Vec<String>] {
        &self.chains
    }

    /// **Private.** Normalised names whose built-in chains the project turned off.
    pub(crate) fn chains_off(&self) -> &[String] {
        &self.chains_off
    }

    /// The built-in dictionary, parsed once and shared.
    pub fn builtin() -> Arc<Dictionary> {
        static CACHE: OnceLock<Arc<Dictionary>> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                Arc::new(
                    Dictionary::parse(BUILTIN)
                        .unwrap_or_else(|e| panic!("built-in dictionary: {e}")),
                )
            })
            .clone()
    }

    /// Load-time warnings, e.g. two keys that normalise alike (the later one wins).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Number of keys, exact and suffix.
    pub fn len(&self) -> usize {
        self.exact.len() + self.suffix.len()
    }

    /// Whether the dictionary has no keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether some key matches `field`.
    pub fn contains(&self, field: &str) -> bool {
        self.lookup(field).is_some()
    }

    /// **Private.** The entry for `field`: an exact key, else the longest whole-word suffix key.
    /// A short ASCII name (nearly all of them) is matched on the stack without allocating:
    /// this runs for every field of every generated body.
    pub(crate) fn lookup(&self, field: &str) -> Option<&[Gen]> {
        let mut buf = [0u8; STACK_NAME];
        let mut starts = [0u8; STACK_NAME];
        let Some((len, nwords)) = split_ascii(field, &mut buf, &mut starts) else {
            return self.lookup_alloc(field);
        };
        // `buf` holds lower-case ASCII only
        let key = |from: usize| std::str::from_utf8(&buf[from..len]).unwrap_or("");
        if let Some(g) = self.exact.get(key(0)) {
            return Some(g);
        }
        (1..=nwords)
            .rev()
            .find_map(|k| self.suffix.get(key(usize::from(starts[nwords - k]))))
            .map(Vec::as_slice)
    }

    /// **Private.** [`Dictionary::lookup`] for a name the stack path cannot take (non-ASCII or long).
    fn lookup_alloc(&self, field: &str) -> Option<&[Gen]> {
        if let Some(g) = self.exact.get(&norm(field)) {
            return Some(g);
        }
        let w = words(field);
        (1..=w.len())
            .rev()
            .find_map(|k| self.suffix.get(&w[w.len() - k..].concat()))
            .map(Vec::as_slice)
    }
}

/// **Private.** The longest field name [`split_ascii`] takes.
const STACK_NAME: usize = 64;

/// **Private.** [`norm`] and [`words`] in one pass for an ASCII `field` of at most `STACK_NAME`
/// bytes: the normalised name goes into `buf`, the offset in `buf` where each word starts into
/// `starts`; returns `(length, word count)`. `None` when the name does not qualify. The splitting
/// rule is `words`' (checked against it in the tests).
fn split_ascii(
    field: &str,
    buf: &mut [u8; STACK_NAME],
    starts: &mut [u8; STACK_NAME],
) -> Option<(usize, usize)> {
    let b = field.as_bytes();
    if !field.is_ascii() || b.len() > STACK_NAME {
        return None;
    }
    let (mut len, mut nwords, mut in_word) = (0usize, 0usize, false);
    for (i, &c) in b.iter().enumerate() {
        if c == b'_' || c == b'-' {
            in_word = false;
            continue;
        }
        if c.is_ascii_uppercase() && in_word {
            let prev = b[i - 1];
            let next_lower = b.get(i + 1).is_some_and(u8::is_ascii_lowercase);
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_lower)
            {
                in_word = false;
            }
        }
        if !in_word {
            starts[nwords] = u8::try_from(len).unwrap_or(0);
            nwords += 1;
            in_word = true;
        }
        buf[len] = c.to_ascii_lowercase();
        len += 1;
    }
    Some((len, nwords))
}

#[cfg(test)]
mod tests {
    use super::Dictionary;

    /// The stack path and the allocating path agree on every kind of name.
    #[test]
    fn stack_lookup_matches_the_allocating_one() {
        let d = Dictionary::builtin();
        for name in [
            "id",
            "ID",
            "userId",
            "user_id",
            "user-id",
            "IDCard",
            "lastOrderID",
            "createdAt",
            "created_at",
            "photoUrls",
            "firstName",
            "FIRST_NAME",
            "__x__",
            "a1B",
            "v2Url",
            "",
            "-",
            "zzzUnknownField",
            "HTTPServerURL",
            "naïveName",
        ] {
            assert_eq!(
                d.lookup(name).map(<[_]>::len),
                d.lookup_alloc(name).map(<[_]>::len),
                "{name}"
            );
        }
    }
}
