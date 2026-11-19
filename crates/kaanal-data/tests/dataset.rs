use std::path::{Path, PathBuf};

use kaanal_data::{DataSet, generate};
use serde_json::{Value, json};

/// A fresh empty project directory.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-data-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A minimal valid pack for `tag`, with `country` as its marker.
fn pack(tag: &str, country: &str) -> String {
    format!(
        "version: 1\ntag: {tag}\nname_order: given-family\ncountry: {country}\ncurrency: {{ code: TST, symbol: T }}\nphone: {{ prefix: \"+0\", formats: [\"{{4}}\"] }}\naddress: \"{{number}} {{street}}, {{city}}\"\nfirst_names: [Ann]\nlast_names: [Lee]\nstreets: [Main]\nplaces: [{{ city: Town, postcode: \"{{5}}\" }}]\n"
    )
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn data_set_loads_builtins_and_project_pack_replaces_a_builtin_tag() {
    let dir = scratch("replace");
    write(&dir, "locales/en-US.yaml", &pack("en-US", "Testland"));
    write(&dir, "locales/xx-YY.yaml", &pack("xx-YY", "Nowhere"));
    let ds = DataSet::load(&dir, None).unwrap();
    assert_eq!(ds.options_for("en-US").unwrap().pack.country, "Testland");
    assert_eq!(ds.options_for("xx-YY").unwrap().pack.country, "Nowhere");
    assert_eq!(ds.options_for("en-IN").unwrap().pack.country, "India");
    assert_eq!(
        ds.tags(),
        [
            "de-DE", "en-GB", "en-IN", "en-US", "es-ES", "fr-FR", "pt-BR", "xx-YY"
        ]
    );
}

#[test]
fn unknown_locale_error_lists_available_tags() {
    let dir = scratch("unknown");
    write(&dir, "locales/xx-YY.yaml", &pack("xx-YY", "Nowhere"));
    let e = DataSet::load(&dir, None)
        .unwrap()
        .options_for("ja-JP")
        .unwrap_err();
    assert!(
        e.message.contains("ja-JP")
            && e.message
                .contains("de-DE, en-GB, en-IN, en-US, es-ES, fr-FR, pt-BR, xx-YY"),
        "{e}"
    );
}

#[test]
fn locale_tag_with_path_characters_is_rejected() {
    let dir = scratch("paths");
    // A pack outside `locales/` that a path-built lookup would reach.
    write(&dir, "x.yaml", &pack("x", "Escaped"));
    let ds = DataSet::load(&dir.join("sub"), None).unwrap();
    for tag in ["../../x", "../x", "a/b", "en_IN", "", "en-IN/../en-US"] {
        let e = ds.options_for(tag).unwrap_err();
        assert!(e.message.contains("available"), "{tag}: {e}");
    }
    // A file whose name is not a tag is a load error naming the file, not skipped.
    write(&dir, "locales/en_IN.yaml", &pack("en_IN", "Bad"));
    let e = DataSet::load(&dir, None).unwrap_err();
    assert!(
        e.file.as_ref().unwrap().ends_with("en_IN.yaml") && e.message.contains("locale tag"),
        "{e:?}"
    );
}

#[test]
fn a_pack_tag_must_match_its_file_name() {
    let dir = scratch("mismatch");
    write(&dir, "locales/de-DE.yaml", &pack("fr-FR", "France"));
    let e = DataSet::load(&dir, None).unwrap_err();
    assert!(e.file.as_ref().unwrap().ends_with("de-DE.yaml"), "{e:?}");
    assert_eq!(e.at, ["tag"], "{e:?}");
    assert!(e.message.contains("de-DE"), "{e}");
}

#[test]
fn empty_data_yaml_is_fine_and_missing_locales_dir_is_fine() {
    let dir = scratch("empty");
    write(&dir, "data.yaml", "");
    let ds = DataSet::load(&dir, None).unwrap();
    assert_eq!(ds.warnings(), [] as [String; 0]);
    let o = ds.options_for("en-IN").unwrap();
    assert!(o.dictionary_project.as_ref().unwrap().is_empty());
    // No data.yaml at all, no locales/: built-ins only.
    let ds = DataSet::load(&scratch("bare"), None).unwrap();
    assert!(
        ds.options_for("en-US")
            .unwrap()
            .dictionary_project
            .is_none()
    );
}

#[test]
fn data_file_errors_are_positioned_in_their_file() {
    let dir = scratch("bad-data");
    write(
        &dir,
        "data.yaml",
        "version: 1\nfields:\n  sku: { gen: nope }\n",
    );
    let e = DataSet::load(&dir, None).unwrap_err();
    assert!(e.file.as_ref().unwrap().ends_with("data.yaml"), "{e:?}");
    assert_eq!(e.line, Some(3), "{e:?}");
    assert!(e.message.contains("nope"), "{e}");
    // A pack check serde cannot place names its key path.
    let dir = scratch("bad-pack");
    write(
        &dir,
        "locales/xx-YY.yaml",
        &pack("xx-YY", "X").replace("{number} {street}, {city}", "{nope}"),
    );
    let e = DataSet::load(&dir, None).unwrap_err();
    assert_eq!(e.at, ["address"], "{e:?}");
}

#[test]
fn configured_data_path_is_used_and_a_missing_one_is_an_error_at_data() {
    let dir = scratch("data-path");
    write(
        &dir,
        "conf/mine.yaml",
        "fields:\n  sku: { gen: static, value: X }\n",
    );
    // data.yaml next to the config is ignored when `data:` names another file.
    write(&dir, "data.yaml", "fields:\n  sku: { gen: nope }\n");
    let ds = DataSet::load(&dir, Some("conf/mine.yaml")).unwrap();
    let o = ds.options_for("en-IN").unwrap();
    let s = json!({ "type": "object", "properties": { "sku": { "type": "string" } } });
    assert_eq!(generate(&s, None, 1, &o)["sku"], "X");
    let e = DataSet::load(&dir, Some("conf/missing.yaml")).unwrap_err();
    assert_eq!(
        (e.file.as_ref(), e.at.as_slice()),
        (None, &["data".to_string()][..]),
        "{e:?}"
    );
}

#[test]
fn key_collisions_in_data_yaml_are_warnings() {
    let dir = scratch("collide");
    write(
        &dir,
        "data.yaml",
        "fields:\n  user_email: { gen: email }\n  userEmail: { gen: uuid }\n",
    );
    let ds = DataSet::load(&dir, None).unwrap();
    assert_eq!(ds.warnings().len(), 1, "{:?}", ds.warnings());
    assert!(ds.warnings()[0].contains("data.yaml") && ds.warnings()[0].contains("userEmail"));
}

/// The CLI fixture project: a `ja-JP` pack in native script.
#[test]
fn ja_jp_project_pack_yields_cjk_names_ascii_emails_and_respects_max_length() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal/tests/fixtures");
    let o = DataSet::load(&dir, None)
        .unwrap()
        .options_for("ja-JP")
        .unwrap();
    let s = json!({ "type": "object", "properties": {
        "lastName": { "type": "string", "maxLength": 1 },
        "firstName": { "type": "string" },
        "email": { "type": "string" },
        "currency": { "type": "string" },
    }});
    let mut user_n = false;
    for seed in 0..40 {
        let v = generate(&s, None, seed, &o);
        let last = v["lastName"].as_str().unwrap();
        assert_eq!(last.chars().count(), 1, "{v}");
        assert!(!last.is_ascii(), "{v}");
        assert!(!v["firstName"].as_str().unwrap().is_ascii(), "{v}");
        let email = v["email"].as_str().unwrap();
        assert!(email.is_ascii() && email.ends_with("@example.com"), "{v}");
        user_n |= email.contains("user");
        assert_eq!(v["currency"], Value::from("JPY"));
    }
    assert!(user_n, "a name without latin: gives user<n>");
}

/// YAML text as JSON, for schema checks.
fn yaml(src: &str) -> Value {
    serde_yaml_ng::from_str(src).unwrap()
}

#[test]
fn schemas_accept_the_built_ins_and_reject_a_wrong_entry() {
    let dict = jsonschema::validator_for(&kaanal_data::dictionary_schema()).unwrap();
    let builtin = yaml(include_str!("../data/dictionary.yaml"));
    assert!(
        dict.is_valid(&builtin),
        "{:?}",
        dict.iter_errors(&builtin).next()
    );
    // One generator or a list of them (R1.50 b); an unknown generator or parameter is not valid.
    assert!(dict.is_valid(
        &json!({ "fields": { "id": [{ "gen": "uuid" }, { "gen": "integer", "min": 1 }] } })
    ));
    assert!(!dict.is_valid(&json!({ "fields": { "id": { "gen": "nope" } } })));
    assert!(!dict.is_valid(&json!({ "fields": { "id": { "gen": "uuid", "min": 1 } } })));
    assert!(!dict.is_valid(&json!({ "fields": { "id": [] } })));
    let pack = jsonschema::validator_for(&kaanal_data::pack_schema()).unwrap();
    for src in [
        include_str!("../data/locales/en-IN.yaml"),
        include_str!("../data/locales/en-US.yaml"),
    ] {
        let v = yaml(src);
        assert!(pack.is_valid(&v), "{:?}", pack.iter_errors(&v).next());
    }
    assert!(!pack.is_valid(&yaml(&pack_without_streets())));
}

fn pack_without_streets() -> String {
    pack("xx-YY", "X").replace("streets: [Main]\n", "")
}

#[test]
fn symlinks_in_locales_are_skipped_with_a_warning() {
    let dir = scratch("symlink");
    write(&dir, "elsewhere/xx-YY.yaml", &pack("xx-YY", "Linked"));
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    let (target, link) = (
        dir.join("elsewhere/xx-YY.yaml"),
        dir.join("locales/xx-YY.yaml"),
    );
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&target, &link);
    if made.is_err() {
        // Windows without the symlink privilege: nothing to test here.
        return;
    }
    let ds = DataSet::load(&dir, None).unwrap();
    assert!(ds.options_for("xx-YY").is_err(), "the link is not followed");
    assert!(
        ds.warnings()
            .iter()
            .any(|w| w.contains("xx-YY.yaml (symlinks are not followed)")),
        "{:?}",
        ds.warnings()
    );
}

#[test]
fn more_than_64_pack_files_is_an_error_at_the_directory() {
    let dir = scratch("too-many");
    for i in 0..65 {
        write(&dir, &format!("locales/x{i:02}.yaml"), "");
    }
    let e = DataSet::load(&dir, None).unwrap_err();
    assert!(e.file.as_ref().unwrap().ends_with("locales"), "{e:?}");
    assert!(e.message.contains("65 pack files; at most 64"), "{e}");
}

#[test]
fn yml_and_upper_case_yaml_files_are_skipped_with_a_warning() {
    let dir = scratch("yml");
    write(&dir, "locales/xx-YY.yml", &pack("xx-YY", "A"));
    write(&dir, "locales/zz-ZZ.YAML", &pack("zz-ZZ", "B"));
    write(&dir, "locales/notes.txt", "ignored silently");
    let ds = DataSet::load(&dir, None).unwrap();
    assert_eq!(
        ds.tags(),
        [
            "de-DE", "en-GB", "en-IN", "en-US", "es-ES", "fr-FR", "pt-BR"
        ]
    );
    let w = ds.warnings();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w.iter().all(|w| w.contains("pack files must end in .yaml")),
        "{w:?}"
    );
}

#[test]
fn tags_that_differ_only_in_case_are_rejected_and_lookups_hint() {
    let dir = scratch("case");
    write(&dir, "locales/en-us.yaml", &pack("en-us", "Lower"));
    let e = DataSet::load(&dir, None).unwrap_err();
    assert!(e.file.as_ref().unwrap().ends_with("en-us.yaml"), "{e:?}");
    assert!(
        e.message.contains("`en-us`") && e.message.contains("`en-US`"),
        "{e}"
    );
    assert_eq!((e.at.as_slice(), e.value), (&["tag".to_string()][..], true));
    let e = DataSet::load(&scratch("case-hint"), None)
        .unwrap()
        .options_for("en-us")
        .unwrap_err();
    assert!(e.message.contains("did you mean `en-US`?"), "{e}");
}

#[test]
fn a_configured_data_path_that_is_not_a_file_is_an_error_at_data() {
    let dir = scratch("data-dir");
    std::fs::create_dir_all(dir.join("conf")).unwrap();
    let e = DataSet::load(&dir, Some("conf")).unwrap_err();
    assert_eq!(
        (e.file, e.at, e.value),
        (None, vec!["data".to_string()], true)
    );
    assert!(e.message.contains("is not a file"), "{}", e.message);
}

#[test]
fn files_lists_what_load_reads() {
    let dir = scratch("files");
    assert!(
        DataSet::files(&dir, None).unwrap().is_empty(),
        "nothing yet"
    );
    write(&dir, "locales/xx-YY.yaml", &pack("xx-YY", "Nowhere"));
    write(&dir, "locales/notes.txt", "not a pack\n");
    write(
        &dir,
        "data.yaml",
        "fields:\n  sku: { gen: static, value: X }\n",
    );
    write(&dir, "conf/mine.yaml", "fields: {}\n");
    assert_eq!(
        DataSet::files(&dir, None).unwrap(),
        [dir.join("locales/xx-YY.yaml"), dir.join("data.yaml")]
    );
    // a configured `data:` replaces `data.yaml`
    assert_eq!(
        DataSet::files(&dir, Some("conf/mine.yaml")).unwrap(),
        [dir.join("locales/xx-YY.yaml"), dir.join("conf/mine.yaml")]
    );
}
