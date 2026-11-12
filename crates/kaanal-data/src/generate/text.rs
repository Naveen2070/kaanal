//! Text values: names, addresses, phones, emails, ids, base64, dates and the sentence builder.

use super::dist::{bands, money, pick_weighted, recency_secs};
use super::{
    BASE_EPOCH, ChaCha8Rng, Gen, IndexedRandom, IntDist, MAX_MIN_LENGTH, Name, NameOrder, Pack,
    Place, RngExt, SeedableRng, TextKind, Value, count, digit_count, entry_range, fill, int_for,
    json, norm, number_for, numeric, stable_hash, template_key, value_fits, words,
};

/// **Private.** Runs one dictionary generator for field `f` of scalar type `ty` (it fits).
/// Numbers clamp to the schema's bounds; text is fitted to its lengths in characters,
/// except well-formed values (email, phone, url, uuid, timestamps; R4.5).
pub(super) fn run(
    g: &Gen,
    s: &Value,
    f: &str,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    pack: &Pack,
) -> Value {
    let (up, down) = (|x: f64| x + 1000.0, |x: f64| x - 1000.0);
    let v = match g {
        Gen::Integer { min, max, dist } => {
            let (lo, hi) = entry_range(
                *min,
                *max,
                (0, 1000),
                |x: i64| x.saturating_add(1000),
                |x: i64| x.saturating_sub(1000),
            );
            let n = if *dist == IntDist::Bands {
                json!(banded(s, rng, (lo, hi)))
            } else {
                json!(int_for(s, rng, (lo, hi)))
            };
            // a `number` field may have bounds no whole number fits (0.1..0.9): then a float
            if ty == "number" && !value_fits(&n, s, ty) {
                #[expect(clippy::cast_precision_loss, reason = "a default range only")]
                return number_for(s, rng, (lo as f64, hi as f64), 2);
            }
            return n;
        }
        Gen::Money {
            min,
            max,
            dist,
            style,
        } => {
            let range = entry_range(*min, *max, (99.0, 4999.0), up, down);
            return money(s, ty, rng, range, (*dist, *style));
        }
        Gen::Latitude { min, max } | Gen::Longitude { min, max } => {
            let range = geo_range(pack, matches!(g, Gen::Latitude { .. }));
            return numeric(s, ty, rng, entry_range(*min, *max, range, up, down), 6);
        }
        // user-written values are sent as written, never cut or padded (R5.7); a project
        // value outside the lengths is warned at startup, a built-in one never chosen
        Gen::Pick { from, weights } => {
            let fit: Vec<&Value> = from.iter().filter(|v| value_fits(v, s, ty)).collect();
            return match weights {
                Some(w) => pick_weighted(&fit, &w.0, rng).clone(),
                None => (*fit.choose(rng).expect("`fits` found one")).clone(),
            };
        }
        Gen::Static { value } => return value.clone(),
        // well-formed: no length fitting (R4.5)
        Gen::Email {} => return json!(email(rng, group, pack)),
        Gen::Phone {} => return json!(phone(rng, pack)),
        Gen::Url {} => return json!(url(f, rng)),
        Gen::Uuid {} => return json!(uuid(rng)),
        Gen::Timestamp { days } => return json!(date_time(rng, days.unwrap_or(DEFAULT_DAYS))),
        Gen::Date { days, years } => {
            if let Some([lo, hi]) = years {
                // a birth date: an age from the weighted bands, then a day inside that year
                let age = bands(i64::from(*lo), i64::from(*hi), rng);
                return json!(day_text(birth_day(age, rng)));
            }
            return json!(date_time(rng, days.unwrap_or(DEFAULT_DAYS))[..10].to_string());
        }
        Gen::FirstName {} => person(group, pack).0.name.clone(),
        Gen::LastName {} => person(group, pack).1.name.clone(),
        Gen::FullName {} => {
            let (given, family) = person(group, pack);
            match pack.name_order {
                NameOrder::GivenFamily => format!("{} {}", given.name, family.name),
                NameOrder::FamilyGiven => format!("{} {}", family.name, given.name),
            }
        }
        Gen::Username {} => username(rng, group, pack),
        Gen::City {} => place(group, pack).city.clone(),
        // a stateless locale answers with the city (design 11a rule 4)
        Gen::State {} => {
            let p = place(group, pack);
            p.state.as_ref().unwrap_or(&p.city).clone()
        }
        Gen::Postcode {} => postcode(group, pack),
        Gen::Country {} => pack.country.clone(),
        Gen::Street {} => street(group, pack).to_string(),
        Gen::Address {} => address(rng, group, pack),
        Gen::Currency {} => pack.currency.code.clone(),
        Gen::Locale {} => pack.tag.clone(),
        Gen::Language {} => pack.tag.split('-').next().unwrap_or(&pack.tag).to_string(),
        Gen::Text { kind } => match kind {
            TextKind::Sentence => sentence(rng),
            TextKind::Paragraph => {
                let n = rng.random_range(3..=5);
                (0..n).map(|_| sentence(rng)).collect::<Vec<_>>().join(" ")
            }
        },
        Gen::Template { value } => {
            return json!(
                fill(value, |k| {
                    let (letters, n) = template_key(k)?;
                    Some(if letters {
                        (0..n)
                            .map(|_| char::from(b'A' + rng.random_range(0..26u8)))
                            .collect()
                    } else {
                        digits(rng, n)
                    })
                })
                .expect("templates are checked when the dictionary loads")
            );
        }
    };
    json!(fit_length(v, s))
}

/// **Private.** The default `(min, max)` of a `latitude` (`lat`) or `longitude` field: the
/// pack's `geo` block, else the whole globe.
pub(super) fn geo_range(pack: &Pack, lat: bool) -> (f64, f64) {
    let [lo, hi] = match (pack.geo, lat) {
        (Some(g), true) => g.lat,
        (Some(g), false) => g.lon,
        (None, true) => [-90.0, 90.0],
        (None, false) => [-180.0, 180.0],
    };
    (lo, hi)
}

/// **Private.** `https://example.com/<stem>/<n>`, the stem being the field name without a
/// trailing `url` (`avatarUrl` → `avatar`); a bare `url` has no empty segment.
pub(super) fn url(f: &str, rng: &mut ChaCha8Rng) -> String {
    let n = rng.random_range(1..=9999);
    let f = norm(f);
    match f.strip_suffix("url").unwrap_or(&f) {
        "" => format!("https://example.com/{n}"),
        stem => format!("https://example.com/{stem}/{n}"),
    }
}

/// **Private.** A phone number in one of the pack's formats.
pub(super) fn phone(rng: &mut ChaCha8Rng, pack: &Pack) -> String {
    let format = pack
        .phone
        .formats
        .choose(rng)
        .expect("formats is non-empty");
    format!("{} {}", pack.phone.prefix, digits_template(format, rng))
}

/// **Private.** The object's address from the pack's template.
pub(super) fn address(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let p = place(group, pack);
    fill(&pack.address, |k| {
        Some(match k {
            "number" => rng.random_range(1..=999).to_string(),
            "street" => street(group, pack).to_string(),
            "city" => p.city.clone(),
            "state" => p.state.clone().unwrap_or_default(),
            "postcode" => postcode(group, pack),
            _ => return None,
        })
    })
    .expect("the address template is checked when the pack loads")
}

/// **Private.** Index into a list of `len` items from the object's group seed and a tag.
pub(super) fn group_index(group: u64, tag: &[u8], len: usize) -> usize {
    let i = stable_hash(&[&group.to_le_bytes(), tag]) % len as u64;
    usize::try_from(i).expect("i < len")
}

/// **Private.** The object's street: `street` and the address's `{street}` agree.
pub(super) fn street(group: u64, pack: &Pack) -> &str {
    &pack.streets[group_index(group, b"street", pack.streets.len())]
}

/// **Private.** The place row of the object a field belongs to: same object, same place.
pub(super) fn place(group: u64, pack: &Pack) -> &Place {
    &pack.places[group_index(group, b"place", pack.places.len())]
}

/// **Private.** The object's postcode: the place's pattern with `{n}` filled from the
/// group seed, so `postcode` and `address` agree.
pub(super) fn postcode(group: u64, pack: &Pack) -> String {
    let rng = &mut ChaCha8Rng::seed_from_u64(stable_hash(&[&group.to_le_bytes(), b"postcode"]));
    digits_template(&place(group, pack).postcode, rng)
}

/// **Private.** A `{n}`-digit pattern (phone format, postcode) filled with seeded digits.
pub(super) fn digits_template(pattern: &str, rng: &mut ChaCha8Rng) -> String {
    fill(pattern, |k| digit_count(k).map(|n| digits(rng, n)))
        .expect("patterns are checked when the pack loads")
}

/// **Private.** The (given, family) name of the person an object describes: same
/// object, same person, so `firstName`, `lastName`, `name` and `email` agree.
pub(super) fn person(group: u64, pack: &Pack) -> (&Name, &Name) {
    (
        &pack.first_names[group_index(group, b"first", pack.first_names.len())],
        &pack.last_names[group_index(group, b"last", pack.last_names.len())],
    )
}

/// **Private.** `first.lastNN@example.com` for the object's person — always a reserved domain (rule R5.7).
/// Local part is [`Pack::ascii_local`]: `latin:` form, else folded, else `user<n>`.
pub(super) fn email(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let (first, last) = person(group, pack);
    let n = rng.random_range(1..=99);
    let (first, last) = (pack.ascii_local(first, n), pack.ascii_local(last, n));
    let fallback = format!("user{n}");
    if first == fallback && last == fallback {
        // both names fell back: one `user<n>`, not `user7.user77`
        return format!("{fallback}@example.com");
    }
    format!("{first}.{last}{n}@example.com")
}

/// **Private.** `first_lastNN` for the object's person, ASCII like the email's local part.
pub(super) fn username(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let (first, last) = person(group, pack);
    let n = rng.random_range(1..=99);
    let (first, last) = (pack.ascii_local(first, n), pack.ascii_local(last, n));
    if first == format!("user{n}") && last == first {
        return first;
    }
    format!("{first}_{last}{n}")
}

/// **Private.** A random UUID with the v4 version and variant bits set.
pub(super) fn uuid(rng: &mut ChaCha8Rng) -> String {
    use std::fmt::Write as _;
    let mut b: [u8; 16] = rng.random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().fold(String::with_capacity(32), |mut s, x| {
        write!(s, "{x:02x}").expect("writing to a String cannot fail");
        s
    });
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// **Private.** Standard base64 with padding.
pub(super) fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = c.iter().fold(0u32, |n, x| n << 8 | u32::from(*x)) << (8 * (3 - c.len()));
        for i in 0..=c.len() {
            out.push(char::from(A[(n >> (18 - 6 * i) & 63) as usize]));
        }
        out.extend(std::iter::repeat_n('=', 3 - c.len()));
    }
    out
}

/// **Private.** `n` random decimal digits (phone numbers).
pub(super) fn digits(rng: &mut ChaCha8Rng, n: usize) -> String {
    (0..n)
        .map(|_| char::from(b'0' + rng.random_range(0..10u8)))
        .collect()
}

/// **Private.** A 6–12 word lorem sentence, capitalised, ending with a full stop.
pub(super) fn sentence(rng: &mut ChaCha8Rng) -> String {
    let n = rng.random_range(6..=12);
    let mut s: Vec<&str> = (0..n).map(|_| words::pick(words::WORDS, rng)).collect();
    let first = s[0][..1].to_uppercase() + &s[0][1..];
    s[0] = "";
    format!("{first}{}.", s.join(" "))
}

/// **Private.** The window of a timestamp or date, in days before [`BASE_EPOCH`].
pub(super) const DEFAULT_DAYS: u32 = 365;

/// **Private.** An RFC 3339 UTC timestamp within the `days` before [`BASE_EPOCH`], leaning
/// recent: two in three fall in the last third of the window.
pub(super) fn date_time(rng: &mut ChaCha8Rng, days: u32) -> String {
    let t = BASE_EPOCH - recency_secs(i64::from(days.max(1)) * 86_400, rng);
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// **Private.** Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).expect("day in 1..=31");
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).expect("month in 1..=12");
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// **Private.** Applies `maxLength` (truncate) and `minLength` (pad with `x` in one pass,
/// capped at [`MAX_MIN_LENGTH`]) to a plain string. Never called on formatted or well-formed values.
pub(super) fn fit_length(mut v: String, s: &Value) -> String {
    if let Some(max) = count(s, "maxLength", false) {
        v = v
            .chars()
            .take(usize::try_from(max).unwrap_or(usize::MAX))
            .collect();
    }
    if let Some(min) = count(s, "minLength", true) {
        let min = usize::try_from(min.min(MAX_MIN_LENGTH)).expect("capped at MAX_MIN_LENGTH");
        let pad = min.saturating_sub(v.chars().count());
        v.extend(std::iter::repeat_n('x', pad));
    }
    v
}

/// **Private.** (year, month, day) → days since 1970-01-01. The inverse of [`civil_from_days`].
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// **Private.** An integer from [`bands`](super::dist::bands) inside the schema's bounds (the
/// entry's range where the schema has none); plain `int_for` when the bounds leave no range.
fn banded(s: &Value, rng: &mut ChaCha8Rng, (lo, hi): (i64, i64)) -> i64 {
    let lo2 = super::numbers::int_bound(s, "minimum", true).unwrap_or(lo);
    let hi2 = super::numbers::int_bound(s, "maximum", false).unwrap_or(hi);
    if lo2 <= hi2 {
        super::dist::bands(lo2, hi2, rng)
    } else {
        int_for(s, rng, (lo, hi))
    }
}

/// **Private.** Days since 1970-01-01 of a birthday that makes someone exactly `age` whole years
/// old on the base date: a day in (base - (age + 1) years, base - age years], from `rng`.
pub(crate) fn birth_day(age: i64, rng: &mut ChaCha8Rng) -> i64 {
    let year = civil_from_days(BASE_EPOCH.div_euclid(86_400)).0;
    let (from, to) = (
        days_from_civil(year - age - 1, 1, 1),
        days_from_civil(year - age, 1, 1),
    );
    from + 1 + rng.random_range(0..to - from)
}

/// **Private.** `YYYY-MM-DD` for a count of days since 1970-01-01.
fn day_text(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}
