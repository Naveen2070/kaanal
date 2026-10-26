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
    pub(crate) fn lookup(&self, field: &str) -> Option<&[Gen]> {
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
