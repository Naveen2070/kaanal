//! `pattern` support (D2 spec section 6): an in-house sampler over the `regex-syntax` HIR.
//!
//! `rand_regex` is not used: it cannot restrict classes to ASCII, rejects anchors in older
//! releases and couples this crate to a `rand` version. The sampler draws only from the field's
//! seeded `ChaCha8Rng` with integer ranges, so output is the same on every OS. "Does a value
//! match" is decided by the validator's own engine (a one-keyword `jsonschema` schema), so the
//! generator and the self-check never disagree (the `regex` crate's `\w` is Unicode, the
//! validator's is ASCII).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use regex_syntax::hir::{Class, Hir, HirKind};
use serde_json::{Value, json};

use super::{child_seed, count};

/// **Private.** Longest string a pattern may be asked to produce; a pattern whose shortest match
/// is longer is unsupported (`(a{1000}){1000}`).
const MAX_LEN: usize = 1024;

/// **Private.** Extra repeats an open or wide repeat draws beyond its minimum.
const EXTRA_REPEATS: usize = 8;

/// A parsed pattern that can sample matching strings and test a candidate.
pub(crate) struct Compiled {
    hir: Hir,
    matcher: jsonschema::Validator,
}

type Entry = Arc<Result<Compiled, String>>;

/// The compiled form of `p`, parsed once per distinct pattern for the life of the process (the
/// mock path never recompiles). `Err` carries why the pattern is unsupported.
pub(crate) fn cached(p: &str) -> Entry {
    // ponytail: unbounded map keyed by pattern text; a spec has a few dozen patterns.
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let mut map = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = map.get(p) {
        return entry.clone();
    }
    let entry = Arc::new(compile(p));
    map.insert(p.to_string(), entry.clone());
    entry
}

/// Parses `p`. `Err` for syntax the sampler does not support: lookaround and back-references
/// (`regex-syntax` rejects them), or a shortest match over [`MAX_LEN`] characters.
pub(crate) fn compile(p: &str) -> Result<Compiled, String> {
    let hir = regex_syntax::Parser::new()
        .parse(p)
        .map_err(|e| format!("lookaround, back-reference or invalid syntax: {e}"))?;
    if min_len(&hir) > MAX_LEN {
        return Err(format!("shortest match is over {MAX_LEN} characters"));
    }
    let matcher = jsonschema::options()
        .build(&json!({ "type": "string", "pattern": p }))
        .map_err(|e| format!("the validator rejects it: {e}"))?;
    Ok(Compiled { hir, matcher })
}

impl Compiled {
    /// A string matching the pattern, aiming at `min..=max` characters: `max` bounds how many
    /// repeats open repetitions draw, and an open repetition adds what is still needed to reach
    /// `min`. A mandatory part can still fall outside, so the caller checks the length.
    pub(crate) fn sample(
        &self,
        rng: &mut ChaCha8Rng,
        (min, max): (usize, Option<usize>),
    ) -> String {
        let mut out = Out::default();
        let want = Want {
            min,
            budget: max.map_or(MAX_LEN, |m| m.min(MAX_LEN)),
        };
        emit(&self.hir, rng, want, &mut out);
        out.text
    }

    /// Whether `s` satisfies the pattern, by the validator's semantics.
    pub(crate) fn is_match(&self, s: &str) -> bool {
        self.matcher.is_valid(&Value::String(s.to_string()))
    }
}

/// **Private.** Fewest characters any match needs (saturating).
fn min_len(h: &Hir) -> usize {
    match h.kind() {
        HirKind::Empty | HirKind::Look(_) => 0,
        HirKind::Literal(b) => String::from_utf8_lossy(&b.0).chars().count(),
        HirKind::Class(_) => 1,
        HirKind::Capture(c) => min_len(&c.sub),
        HirKind::Repetition(r) => min_len(&r.sub).saturating_mul(r.min as usize),
        HirKind::Concat(v) => v.iter().fold(0, |a, x| a.saturating_add(min_len(x))),
        HirKind::Alternation(v) => v.iter().map(min_len).min().unwrap_or(0),
    }
}

/// **Private.** The output so far and its length in characters (kept as it grows, so a long
/// match is not recounted at every repetition).
#[derive(Default)]
struct Out {
    text: String,
    chars: usize,
}

impl Out {
    fn push_str(&mut self, s: &str) {
        self.chars += s.chars().count();
        self.text.push_str(s);
    }

    fn push(&mut self, c: char) {
        self.chars += 1;
        self.text.push(c);
    }
}

/// **Private.** The length a whole match is aimed at: at least `min`, at most `budget`.
#[derive(Clone, Copy)]
struct Want {
    min: usize,
    budget: usize,
}

/// **Private.** Appends one match of `h`.
fn emit(h: &Hir, rng: &mut ChaCha8Rng, want: Want, out: &mut Out) {
    match h.kind() {
        // anchors and boundaries match the empty string
        HirKind::Empty | HirKind::Look(_) => {}
        HirKind::Literal(b) => out.push_str(&String::from_utf8_lossy(&b.0)),
        HirKind::Class(c) => out.push(pick_char(c, rng)),
        HirKind::Capture(c) => emit(&c.sub, rng, want, out),
        HirKind::Concat(v) => v.iter().for_each(|x| emit(x, rng, want, out)),
        HirKind::Alternation(v) => emit(&v[rng.random_range(0..v.len())], rng, want, out),
        HirKind::Repetition(r) => {
            let min = r.min as usize;
            let each = min_len(&r.sub).max(1);
            // extra repeats beyond the minimum: at most 8, at most what the budget still holds
            let room = (want.budget.saturating_sub(out.chars) / each).saturating_sub(min);
            let widest = r
                .max
                .map_or(usize::MAX, |m| m as usize)
                .saturating_sub(min)
                .min(EXTRA_REPEATS)
                .min(room);
            let mut n = min + rng.random_range(0..=widest);
            if r.max.is_none() {
                // an open repetition makes up the length `min` still asks for (`[a-z]+`, 10+)
                let needed = want.min.saturating_sub(out.chars).div_ceil(each);
                n = n.max(needed.min(min + room));
            }
            for _ in 0..n {
                emit(&r.sub, rng, want, out);
            }
        }
    }
}

/// **Private.** One character of a class, ASCII only: printable ASCII if the class has any
/// (`.` and `[^a]` give letters, not control characters), else any ASCII, else the class's own
/// characters (`\p{Han}`).
fn pick_char(c: &Class, rng: &mut ChaCha8Rng) -> char {
    let ranges: Vec<(u32, u32)> = match c {
        Class::Unicode(u) => u
            .iter()
            .map(|r| (r.start() as u32, r.end() as u32))
            .collect(),
        Class::Bytes(b) => b
            .iter()
            .map(|r| (u32::from(r.start()), u32::from(r.end())))
            .collect(),
    };
    let clip = |lo: u32, hi: u32| -> Vec<(u32, u32)> {
        ranges
            .iter()
            .filter(|&&(a, b)| a.max(lo) <= b.min(hi))
            .map(|&(a, b)| (a.max(lo), b.min(hi)))
            .collect()
    };
    let mut chosen = clip(0x20, 0x7e);
    if chosen.is_empty() {
        chosen = clip(0, 0x7f);
    }
    if chosen.is_empty() {
        chosen = ranges;
    }
    let total: u32 = chosen.iter().map(|&(a, b)| b - a + 1).sum();
    let mut i = rng.random_range(0..total.max(1));
    for (a, b) in chosen {
        if i <= b - a {
            return char::from_u32(a + i).unwrap_or('?');
        }
        i -= b - a + 1;
    }
    '?'
}

/// **Private.** Candidates tried per field before giving up on `minLength`/`maxLength`.
const TRIES: usize = 8;

/// **Private.** The first of [`TRIES`] samples (seeds `child_seed(seed, "pattern", k)`) that
/// matches and has between `minLength` and `maxLength` characters. `fit_length` is never applied
/// to a pattern value (it would break the match).
fn fitting(c: &Compiled, s: &Value, seed: u64) -> Option<String> {
    let (min, max) = (count(s, "minLength", true), count(s, "maxLength", false));
    (0..TRIES).find_map(|k| {
        let mut rng = ChaCha8Rng::seed_from_u64(child_seed(seed, "pattern", k));
        let cand = c.sample(
            &mut rng,
            (
                min.and_then(|m| usize::try_from(m).ok()).unwrap_or(0),
                max.and_then(|m| usize::try_from(m).ok()),
            ),
        );
        let n = u64::try_from(cand.chars().count()).unwrap_or(u64::MAX);
        (min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m) && c.is_match(&cand))
            .then_some(cand)
    })
}

/// A free string `v` for a field with `pattern` `p` (schema `s`): kept when it already matches,
/// else regenerated from the pattern. When the pattern is unsupported or no candidate fits the
/// length bounds, `v` stays (the startup warning says so). Pinned values never get here.
pub(super) fn with_pattern(v: Value, p: &str, s: &Value, rng: &mut ChaCha8Rng) -> Value {
    let entry = cached(p);
    let Ok(c) = entry.as_ref() else { return v };
    if v.as_str().is_some_and(|t| c.is_match(t)) {
        return v;
    }
    fitting(c, s, rng.random::<u64>()).map_or(v, Value::String)
}

/// Why `pattern` on a field with schema `s` cannot be honoured, or `None` when it can.
pub(crate) fn problem(p: &str, s: &Value) -> Option<String> {
    match cached(p).as_ref() {
        Err(_) => Some(format!(
            "pattern {p} is not supported (lookaround, back-reference or too large); generated values may not match"
        )),
        // 4 seeds x 8 candidates: if none fits, the pattern and the lengths really disagree
        Ok(c) => (0..4).all(|seed| fitting(c, s, seed).is_none()).then(|| {
            format!("pattern {p} cannot be met within minLength/maxLength; generated values ignore the pattern")
        }),
    }
}
