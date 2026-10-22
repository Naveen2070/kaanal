use std::sync::Arc;

use kaanal_data::{Options, Pack, builtin_tags, generate};
use serde_json::json;

/// A small stateless pack; `{address}` and `{streets}` are filled per test.
fn de_pack(address: &str, streets: &str) -> String {
    format!(
        "version: 1
tag: de-DE
name_order: given-family
country: Deutschland
currency: {{ code: EUR, symbol: \"€\" }}
phone: {{ prefix: \"+49\", formats: [\"{{3}} {{7}}\"] }}
address: \"{address}\"
first_names:
  - {{ name: Jürgen, latin: Juergen }}
  - Anna
last_names: [Müller, Schmidt]
{streets}places:
  - {{ city: Berlin, postcode: \"10{{3}}\" }}
"
    )
}

const STREETS: &str = "streets: [Hauptstraße]\n";

#[test]
fn en_in_builtin_parses_and_has_required_blocks() {
    assert!(builtin_tags().contains(&"en-IN"));
    let p = Pack::builtin("en-IN").expect("en-IN is built in");
    assert_eq!(p.tag, "en-IN");
    assert_eq!(p.country, "India");
    assert_eq!(p.currency.code, "INR");
    assert_eq!(p.phone.prefix, "+91");
    assert!(p.places.len() >= 60);
    assert!(p.places.iter().all(|pl| pl.state.is_some()));
    assert!(!p.first_names.is_empty() && !p.last_names.is_empty() && !p.streets.is_empty());
    // the default options use it
    assert!(Arc::ptr_eq(&Options::default().pack, &p));
}

#[test]
fn pack_without_streets_is_an_error_naming_the_block() {
    let e = Pack::parse(&de_pack("{street} {number}, {postcode} {city}", "")).unwrap_err();
    assert!(e.message.contains("streets"), "{e}");
    let e = Pack::parse(&de_pack(
        "{street} {number}, {postcode} {city}",
        "streets: []\n",
    ))
    .unwrap_err();
    assert!(e.message.contains("streets") && e.line.is_some(), "{e}");
}

#[test]
fn non_ascii_latin_form_is_an_error_with_line_and_column() {
    let src = de_pack("{street} {number}, {postcode} {city}", STREETS)
        .replace("  - Anna\n", "  - Anna\n  - name: Jörg\n    latin: Jörg\n");
    let e = Pack::parse(&src).unwrap_err();
    assert_eq!((e.line, e.col), (Some(12), Some(12)), "{e}");
    assert!(e.message.contains("ASCII"), "{e}");
    assert!(!e.message.contains("at line"), "{e}");
}

#[test]
fn state_in_template_with_a_stateless_place_names_the_place() {
    let e = Pack::parse(&de_pack(
        "{street} {number}, {postcode} {city}, {state}",
        STREETS,
    ))
    .unwrap_err();
    assert!(
        e.message.contains("Berlin") && e.message.contains("state"),
        "{e}"
    );
}

#[test]
fn stateless_pack_without_state_in_template_loads() {
    let p = Pack::parse(&de_pack("{street} {number}, {postcode} {city}", STREETS)).unwrap();
    assert_eq!(p.places[0].state, None);
    // and it drives generation
    let opts = Options {
        pack: Arc::new(p),
        ..Options::default()
    };
    let s = json!({ "type": "object", "properties": {
        "country": { "type": "string" }, "currency": { "type": "string" },
        "phone": { "type": "string" }, "postalCode": { "type": "string" },
        "state": { "type": "string" }, "address": { "type": "string" },
        "email": { "type": "string" }
    }});
    for seed in 0..20 {
        let v = generate(&s, None, seed, &opts);
        assert_eq!(v["country"], "Deutschland");
        assert_eq!(v["currency"], "EUR");
        let phone = v["phone"].as_str().unwrap();
        assert!(phone.starts_with("+49 ") && phone.len() == 15, "{phone}");
        let pc = v["postalCode"].as_str().unwrap();
        assert!(pc.starts_with("10") && pc.len() == 5, "{pc}");
        assert_eq!(v["state"], "Berlin", "stateless: the city stands in");
        assert!(
            v["address"]
                .as_str()
                .unwrap()
                .ends_with(&format!(", {pc} Berlin"))
        );
        // ASCII folding of `Müller` is T2; here only the domain is checked
        assert!(
            v["email"].as_str().unwrap().ends_with("@example.com"),
            "{v}"
        );
    }
}

#[test]
fn unknown_builtin_tag_is_none() {
    assert!(Pack::builtin("xx-XX").is_none());
}

#[test]
fn every_builtin_tag_resolves() {
    for tag in builtin_tags() {
        assert_eq!(Pack::builtin(tag).expect(tag).tag, *tag);
    }
}

#[test]
fn en_in_matches_the_lists_it_replaced() {
    // The lists of `words.rs` before M1-D1 (FIRST, LAST, STREETS, CITIES), verbatim.
    const FIRST: &[&str] = &[
        "Aarav",
        "Priya",
        "Arjun",
        "Kavya",
        "Rohan",
        "Ananya",
        "Vikram",
        "Meera",
        "Karthik",
        "Divya",
        "Rahul",
        "Sneha",
        "Aditya",
        "Lakshmi",
        "Siddharth",
        "Pooja",
        "Naveen",
        "Deepa",
        "Varun",
        "Nithya",
    ];
    const LAST: &[&str] = &[
        "Sharma",
        "Iyer",
        "Reddy",
        "Nair",
        "Patel",
        "Gupta",
        "Rao",
        "Menon",
        "Singh",
        "Krishnan",
        "Das",
        "Joshi",
        "Kumar",
        "Pillai",
        "Mehta",
        "Subramanian",
        "Bose",
        "Chatterjee",
        "Shetty",
        "Verma",
    ];
    const STREETS: &[&str] = &[
        "MG Road",
        "Anna Salai",
        "Gandhi Nagar",
        "Nehru Street",
        "Park Avenue",
        "Lake View Road",
        "Temple Street",
        "Station Road",
    ];
    const CITIES: &[(&str, &str, &str)] = &[
        ("Chennai", "Tamil Nadu", "600001"),
        ("Coimbatore", "Tamil Nadu", "641001"),
        ("Madurai", "Tamil Nadu", "625001"),
        ("Bengaluru", "Karnataka", "560001"),
        ("Mumbai", "Maharashtra", "400001"),
        ("Pune", "Maharashtra", "411001"),
        ("Hyderabad", "Telangana", "500001"),
        ("Kochi", "Kerala", "682001"),
        ("Delhi", "Delhi", "110001"),
        ("Kolkata", "West Bengal", "700001"),
    ];
    let p = Pack::builtin("en-IN").unwrap();
    let names = |l: &[kaanal_data::Name]| {
        assert!(l.iter().all(|n| n.latin.is_none()));
        l.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
    };
    // T2 grew the lists; the original entries stay first, in order.
    assert_eq!(names(&p.first_names)[..FIRST.len()], *FIRST);
    assert_eq!(names(&p.last_names)[..LAST.len()], *LAST);
    assert_eq!(p.streets[..STREETS.len()], *STREETS);
    let places: Vec<_> = p
        .places
        .iter()
        .map(|pl| {
            (
                pl.city.as_str(),
                pl.state.as_deref().unwrap(),
                pl.postcode.as_str(),
            )
        })
        .collect();
    assert_eq!(places[..CITIES.len()], *CITIES);
}

const GOOD_ADDRESS: &str = "{street} {number}, {postcode} {city}";

#[test]
fn unknown_address_placeholder_is_an_error() {
    let e = Pack::parse(&de_pack("{street} {house}, {city}", STREETS)).unwrap_err();
    assert!(e.message.starts_with("address:"), "{e}");
}

#[test]
fn bad_digit_patterns_are_errors() {
    for (from, to) in [
        ("\"10{3}\"", "\"10{x}\""),
        ("\"10{3}\"", "\"10{+3}\""),
        ("\"10{3}\"", "\"10{0}\""),
        ("\"10{3}\"", "\"10{21}\""),
        ("[\"{3} {7}\"]", "[\"{3} {7\"]"),
    ] {
        let src = de_pack(GOOD_ADDRESS, STREETS).replace(from, to);
        assert_ne!(src, de_pack(GOOD_ADDRESS, STREETS), "{to}");
        let e = Pack::parse(&src).unwrap_err();
        assert!(e.message.contains("{n}"), "{to}: {e}");
    }
}

#[test]
fn unsupported_version_is_an_error() {
    let src = de_pack(GOOD_ADDRESS, STREETS).replace("version: 1", "version: 2");
    let e = Pack::parse(&src).unwrap_err();
    assert!(e.message.contains("version: 2"), "{e}");
}

#[test]
fn unknown_fields_are_errors_with_position() {
    for (from, to, line) in [
        ("tag: de-DE\n", "tag: de-DE\nflag: x\n", 3),
        ("symbol: \"€\" }", "symbol: \"€\", sign: x }", 5),
        ("Berlin, postcode", "Berlin, zip: x, postcode", 14),
        ("{ name: Jürgen,", "{ nick: J, name: Jürgen,", 9),
    ] {
        let e = Pack::parse(&de_pack(GOOD_ADDRESS, STREETS).replace(from, to)).unwrap_err();
        assert!(e.message.contains("unknown field"), "{to}: {e}");
        assert_eq!(e.line, Some(line), "{to}: {e}");
    }
}

#[test]
fn empty_names_and_latin_forms_are_errors_with_position() {
    for (from, to, line, col) in [
        ("  - Anna\n", "  - \"\"\n", 10, 5),
        ("  - Anna\n", "  - { name: \"\" }\n", 10, 13),
        ("latin: Juergen", "latin: \"\"", 9, 28),
        ("[Müller, Schmidt]", "[Müller, \"\"]", 11, 22),
    ] {
        let e = Pack::parse(&de_pack(GOOD_ADDRESS, STREETS).replace(from, to)).unwrap_err();
        assert!(e.message.contains("empty"), "{to}: {e}");
        assert_eq!((e.line, e.col), (Some(line), Some(col)), "{to}: {e}");
    }
}

#[test]
fn a_leading_bom_is_accepted() {
    // Windows editors add a BOM; YAML would reject it, so it is stripped (as core does, R6.4).
    let src = format!("\u{feff}{}", de_pack(GOOD_ADDRESS, STREETS));
    assert_eq!(Pack::parse(&src).unwrap().tag, "de-DE");
}

fn name(n: &str, latin: Option<&str>) -> kaanal_data::Name {
    kaanal_data::Name {
        name: n.to_string(),
        latin: latin.map(str::to_string),
    }
}

#[test]
fn ascii_local_prefers_latin_then_folds_diacritics_then_user_n() {
    let p = Pack::builtin("en-IN").unwrap();
    assert_eq!(p.ascii_local(&name("José", None), 7), "jose");
    assert_eq!(p.ascii_local(&name("田中", Some("Tanaka")), 7), "tanaka");
    assert_eq!(p.ascii_local(&name("田中", None), 7), "user7");
    for (from, to) in [
        ("Müller", "muller"),
        ("Muñoz", "munoz"),
        ("François", "francois"),
        ("Søren", "soren"),
        ("Łukasz", "lukasz"),
        ("Straße", "strasse"),
        ("Dvořák", "dvorak"),
        ("Őz", "oz"),
        ("Æsir", "aesir"),
    ] {
        assert_eq!(p.ascii_local(&name(from, None), 1), to, "{from}");
    }
}

#[test]
fn ascii_local_keeps_only_lowercase_letters_and_digits() {
    let p = Pack::builtin("en-IN").unwrap();
    assert_eq!(p.ascii_local(&name("O'Brien", None), 1), "obrien");
    assert_eq!(p.ascii_local(&name("Mary-Jane", None), 1), "maryjane");
    assert_eq!(
        p.ascii_local(&name("田中", Some("Tanaka Taro")), 1),
        "tanakataro"
    );
    assert_eq!(
        p.ascii_local(&name("x", Some("O'Brien-Smith")), 1),
        "obriensmith"
    );
    assert_eq!(p.ascii_local(&name("田中", Some("'-")), 4), "user4");
}

#[test]
fn ascii_local_never_panics_and_is_always_ascii() {
    let p = Pack::builtin("en-IN").unwrap();
    for s in [
        "田中太郎",
        "😀😀",
        "e\u{301}\u{301}",
        "\u{301}",
        "مرحبا",
        "א\u{200f}ב",
        "a😀é田",
        "\u{0}",
        "İstanbul",
        "ǅ",
        "ı",
    ] {
        let a = p.ascii_local(&name(s, None), 3);
        assert!(a.is_ascii() && !a.is_empty(), "{s:?} -> {a:?}");
        assert_eq!(a, a.to_lowercase(), "{s:?}");
    }
    // a combining mark after a base letter is dropped, the letter kept
    assert_eq!(p.ascii_local(&name("e\u{301}", None), 3), "e");
}

#[test]
fn en_us_person_has_usd_united_states_plus_one() {
    assert!(builtin_tags().contains(&"en-US"));
    let p = Pack::builtin("en-US").expect("en-US is built in");
    assert_eq!(
        (p.country.as_str(), p.currency.code.as_str()),
        ("United States", "USD")
    );
    assert_eq!(
        (p.currency.symbol.as_str(), p.phone.prefix.as_str()),
        ("$", "+1")
    );
    assert_eq!(p.address, "{number} {street}, {city}, {state} {postcode}");
    assert!(p.places.iter().all(|pl| pl.state.is_some()));
    let opts = Options {
        pack: p,
        ..Options::default()
    };
    let s = json!({ "type": "object", "properties": {
        "country": { "type": "string" }, "currency": { "type": "string" },
        "phone": { "type": "string" }, "zip": { "type": "string" },
        "state": { "type": "string" }, "address": { "type": "string" },
        "email": { "type": "string" }
    }});
    for seed in 0..30 {
        let v = generate(&s, None, seed, &opts);
        assert_eq!(v["country"], "United States");
        assert_eq!(v["currency"], "USD");
        assert!(v["phone"].as_str().unwrap().starts_with("+1 "), "{v}");
        let zip = v["zip"].as_str().unwrap();
        assert!(
            zip.len() == 5 && zip.bytes().all(|b| b.is_ascii_digit()),
            "{zip}"
        );
        let state = v["state"].as_str().unwrap();
        assert!(
            v["address"]
                .as_str()
                .unwrap()
                .ends_with(&format!(", {state} {zip}")),
            "{v}"
        );
        assert!(v["email"].as_str().unwrap().is_ascii());
    }
}

#[test]
fn builtin_packs_meet_the_medium_size_floor() {
    for tag in builtin_tags() {
        let p = Pack::builtin(tag).unwrap();
        assert!(p.first_names.len() >= 150, "{tag} first");
        assert!(p.last_names.len() >= 150, "{tag} last");
        assert!(p.places.len() >= 50, "{tag} places");
        // authored targets (D1.7)
        assert!(
            p.first_names.len() >= 200,
            "{tag} first {}",
            p.first_names.len()
        );
        assert!(
            p.last_names.len() >= 200,
            "{tag} last {}",
            p.last_names.len()
        );
        assert!(p.places.len() >= 60, "{tag} places {}", p.places.len());
        assert!(p.streets.len() >= 20, "{tag} streets");
    }
}

#[test]
fn builtin_lists_have_no_duplicates() {
    use std::collections::HashSet;
    for tag in builtin_tags() {
        let p = Pack::builtin(tag).unwrap();
        let uniq = |what: &str, v: Vec<String>| {
            let n = v.len();
            assert_eq!(
                v.iter().collect::<HashSet<_>>().len(),
                n,
                "{tag}: duplicate {what}"
            );
        };
        uniq(
            "first",
            p.first_names.iter().map(|n| n.name.clone()).collect(),
        );
        uniq(
            "last",
            p.last_names.iter().map(|n| n.name.clone()).collect(),
        );
        uniq("street", p.streets.clone());
        uniq(
            "place",
            p.places
                .iter()
                .map(|pl| format!("{}|{:?}", pl.city, pl.state))
                .collect(),
        );
    }
}

#[test]
fn builtin_packs_are_ascii_clean_for_latin_forms() {
    for tag in builtin_tags() {
        let p = Pack::builtin(tag).unwrap();
        for n in p.first_names.iter().chain(&p.last_names) {
            let a = p.ascii_local(n, 1);
            assert!(
                !a.is_empty() && a.bytes().all(|b| b.is_ascii_lowercase()),
                "{tag}: {n:?} -> {a}"
            );
        }
    }
}

#[test]
fn builtin_postcodes_look_valid_for_the_country() {
    // India: 6 digits, first non-zero; US: 5 digits. `{n}` counts as n digits.
    for (tag, len) in [("en-IN", 6), ("en-US", 5)] {
        for pl in &Pack::builtin(tag).unwrap().places {
            let mut shape = String::new();
            let mut rest = pl.postcode.as_str();
            while let Some(i) = rest.find('{') {
                shape.push_str(&rest[..i]);
                let end = rest.find('}').unwrap();
                shape.push_str(&"0".repeat(rest[i + 1..end].parse().unwrap()));
                rest = &rest[end + 1..];
            }
            shape.push_str(rest);
            assert!(
                shape.len() == len && shape.bytes().all(|b| b.is_ascii_digit()),
                "{tag}: {} {}",
                pl.city,
                pl.postcode
            );
            if tag == "en-IN" {
                assert!(!shape.starts_with('0'), "{tag}: {}", pl.postcode);
            }
        }
    }
}

#[test]
fn builtin_files_are_marked_synthetic() {
    for f in ["en-IN", "en-US"] {
        let src = std::fs::read_to_string(format!("data/locales/{f}.yaml")).unwrap();
        assert!(
            src.starts_with("# synthetic data, authored for Kaanal"),
            "{f}"
        );
    }
}
