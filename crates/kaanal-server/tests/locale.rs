//! Data files and locales reaching the server: per-node options, positioned errors, warnings.

use std::path::{Path, PathBuf};

use kaanal_core::Config;
use kaanal_server::ServeOpts;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-srv-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A spec with one `GET {path}` route returning `{ currency, price }`.
fn spec(path: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo: {{ title: t, version: \"1\" }}\npaths:\n  {path}:\n    get:\n      responses:\n        \"200\":\n          description: ok\n          content:\n            application/json:\n              schema:\n                type: object\n                properties:\n                  currency: {{ type: string }}\n                  price: {{ type: integer }}\n"
    )
}

/// Two services, `india` (project default locale) and `us` (`locale: en-US`), plus `tail`.
fn project(name: &str, tail: &str) -> (PathBuf, String) {
    let dir = scratch(name);
    std::fs::write(dir.join("in.yaml"), spec("/in")).unwrap();
    std::fs::write(dir.join("us.yaml"), spec("/us")).unwrap();
    let src = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  india: {{ type: service, spec: in.yaml }}\n  us: {{ type: service, spec: us.yaml, locale: en-US }}\nedges:\n  - {{ from: gw, to: india }}\n  - {{ from: gw, to: us }}\n{tail}"
    );
    (dir, src)
}

fn check(dir: &Path, src: &str) -> Result<Vec<String>, String> {
    kaanal_server::check(&Config::from_yaml(src).unwrap(), dir).map_err(|e| match &e.file {
        Some(f) => format!(
            "{}:{}",
            f.display(),
            e.in_source(&std::fs::read_to_string(f).unwrap())
        ),
        None => format!("kaanal.yaml:{}", e.in_source(src)),
    })
}

#[tokio::test]
async fn a_node_locale_changes_the_currency_of_its_routes() {
    let (dir, src) = project("currency", "");
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: kaanal_server::new_token(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&Config::from_yaml(&src).unwrap(), &dir, opts)
        .await
        .unwrap();
    let get = |p: &str| {
        let url = format!("http://{}{p}", s.addr);
        async move {
            reqwest::get(url)
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };
    assert_eq!(get("/in").await["currency"], "INR");
    assert_eq!(get("/us").await["currency"], "USD");
}

#[test]
fn unknown_node_locale_is_a_positioned_config_error() {
    let (dir, src) = project("unknown-node", "");
    let src = src.replace("locale: en-US", "locale: de-DE");
    let e = check(&dir, &src).unwrap_err();
    assert!(e.starts_with("kaanal.yaml:5:47: "), "{e}");
    assert!(e.contains("node 'us'") && e.contains("en-IN, en-US"), "{e}");
    // The project default is placed at its own value.
    let (dir, src) = project("unknown-top", "locale: xx-YY\n");
    let e = check(&dir, &src).unwrap_err();
    assert!(e.starts_with("kaanal.yaml:9:9: "), "{e}");
}

#[test]
fn data_file_errors_are_placed_in_their_file() {
    let (dir, src) = project("bad-pack", "");
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(
        dir.join("locales/xx-YY.yaml"),
        "version: 1\ntag: xx-YY\nname_order: given-family\ncountry: X\ncurrency: { code: X, symbol: X }\nphone: { prefix: \"+0\", formats: [\"{4}\"] }\naddress: \"{nope}\"\nfirst_names: [A]\nlast_names: [B]\nstreets: [C]\nplaces: [{ city: D, postcode: \"{5}\" }]\n",
    )
    .unwrap();
    let e = check(&dir, &src).unwrap_err();
    assert!(e.contains("xx-YY.yaml:7:10: address"), "{e}");
    // A missing configured data file is placed at `data:` in kaanal.yaml.
    let (dir, src) = project("missing-data", "data: nope.yaml\n");
    let e = check(&dir, &src).unwrap_err();
    assert!(e.starts_with("kaanal.yaml:9:7: "), "{e}");
}

#[test]
fn dictionary_misfits_and_key_collisions_are_startup_warnings() {
    let (dir, src) = project("warn", "");
    std::fs::write(
        dir.join("data.yaml"),
        "fields:\n  price: { gen: pick, from: [cheap] }\n  user_email: { gen: email }\n  userEmail: { gen: email }\n",
    )
    .unwrap();
    let w = check(&dir, &src).unwrap();
    assert!(
        w.iter()
            .any(|w| w.contains("GET /us .price") && w.contains("does not fit")),
        "{w:?}"
    );
    assert!(
        w.iter()
            .any(|w| w.contains("data.yaml") && w.contains("userEmail")),
        "{w:?}"
    );
}

#[test]
fn a_pack_tag_that_does_not_match_its_file_is_placed_at_the_value() {
    let (dir, src) = project("tag-mismatch", "");
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(
        dir.join("locales/xx-YY.yaml"),
        "version: 1
tag: zz-ZZ
name_order: given-family
country: X
currency: { code: X, symbol: X }
phone: { prefix: \"+0\", formats: [\"{4}\"] }
address: \"{city}\"
first_names: [A]
last_names: [B]
streets: [C]
places: [{ city: D, postcode: \"{5}\" }]
",
    )
    .unwrap();
    let e = check(&dir, &src).unwrap_err();
    assert!(e.contains("xx-YY.yaml:2:6: tag: `zz-ZZ`"), "{e}");
}

/// kaanal-data keeps its own copy of the tag rule (R2.4); both must agree.
#[test]
fn both_locale_tag_checks_agree() {
    for t in [
        "en",
        "en-IN",
        "ja-JP",
        "zh-Hant-TW",
        "sr-Latn-RS",
        "EN-in",
        "gsw-CH",
        "",
        "../../x",
        "a/b",
        "en_IN",
        "en-",
        "-en",
        "e",
        "engl-IN",
        "en-I",
        "en-IN-toolongtag",
        "en IN",
        r"en\IN",
        "..",
        "en-IN.yaml",
        "ü-IN",
        "en--IN",
        "en-IN-x1",
    ] {
        assert_eq!(
            kaanal_core::valid_locale_tag(t),
            kaanal_data::valid_locale_tag(t),
            "{t}"
        );
    }
}
