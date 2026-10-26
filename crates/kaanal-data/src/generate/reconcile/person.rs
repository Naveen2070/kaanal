//! Age and birth date (D2 spec section 4.2): `age` and `dateOfBirth` of one person agree.
//!
//! Whole years from the birth date to the base date (2026-01-01) equal `age`. The free field is
//! the one derived: a free date is placed (day offset from its own seed) inside the year that
//! gives exactly that age; a free `age` is computed from a pinned date, unless that breaks its
//! `minimum`/`maximum`, in which case the object is left as generated.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Value, json};

use super::time::{format, parse};
use super::{can_hold, is_pinned, key_for};
use crate::dataset::Options;
use crate::generate::text::{birth_day, civil_from_days, days_from_civil};
use crate::generate::{BASE_EPOCH, child_seed};

/// **Private.** Names a birth date goes by (normalised).
const DOB: [&str; 4] = ["dateofbirth", "dob", "birthdate", "birthday"];

/// **Private.** Oldest age the rule will place a date for.
const MAX_AGE: i64 = 150;

/// Whether the normalised field name `n` is a birth date.
pub(super) fn is_dob_name(n: &str) -> bool {
    DOB.contains(&n)
}

/// Makes `age` and the birth date of `obj` agree. See the module comment.
pub(super) fn age_dob(
    obj: &mut Map<String, Value>,
    props: &Map<String, Value>,
    seed: u64,
    opts: &Options,
) {
    let Some(age_key) = key_for(obj, "age") else {
        return;
    };
    let Some(age) = obj.get(&age_key).and_then(Value::as_i64) else {
        return;
    };
    let Some(dob_key) = DOB.iter().find_map(|n| key_for(obj, n)) else {
        return;
    };
    let Some((epoch, date_only)) = obj.get(&dob_key).and_then(Value::as_str).and_then(parse) else {
        return;
    };
    let pinned = |k: &str| props.get(k).is_some_and(|ps| is_pinned(ps, k, opts));
    if !pinned(&dob_key) {
        if !(0..=MAX_AGE).contains(&age) {
            return;
        }
        let mut rng =
            ChaCha8Rng::seed_from_u64(child_seed(child_seed(seed, &dob_key, 0), "age", 0));
        // born in (base - (age + 1) years, base - age years]: exactly `age` whole years old
        let day = birth_day(age, &mut rng);
        // the last day of that range is the base date's birthday: only at midnight is it that day
        let year = civil_from_days(BASE_EPOCH.div_euclid(86_400)).0;
        let last = days_from_civil(year - age, 1, 1);
        let time_of_day = if date_only || day == last {
            0
        } else {
            epoch.rem_euclid(86_400)
        };
        let text = Value::String(format(day * 86_400 + time_of_day, date_only));
        if props.get(&dob_key).is_none_or(|ps| can_hold(ps, &text)) {
            obj.insert(dob_key, text);
        }
    } else if !pinned(&age_key) {
        let (year, month, day) = civil_from_days(epoch.div_euclid(86_400));
        let base_year = civil_from_days(BASE_EPOCH.div_euclid(86_400)).0;
        let years = base_year - year - i64::from((month, day) > (1, 1));
        let new = json!(years);
        if years >= 0 && props.get(&age_key).is_none_or(|ps| can_hold(ps, &new)) {
            obj.insert(age_key, new);
        }
    }
}
