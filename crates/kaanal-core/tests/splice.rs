//! `kaanal_core::splice`: edits to a config text that change only the bytes they touch.

use kaanal_core::PathSeg;
use kaanal_core::PathSeg::{Index, Key};
use kaanal_core::splice::{Edit, SpliceError, splice};

fn p(parts: &[&str]) -> Vec<PathSeg> {
    parts
        .iter()
        .map(|s| s.parse().map_or_else(|_| Key((*s).to_string()), Index))
        .collect()
}

const SRC: &str = "version: 1
# the front door and its database
nodes:
  nginx: { type: gateway, proc: 1ms }   # entry
  db:    { type: database, ops: { read: 1ms } }

edges:
  - { from: nginx, to: db, net: 1ms }
# end of file
";

fn set(path: &[&str], value: &str) -> Edit {
    Edit::Set {
        path: p(path),
        value: value.into(),
    }
}

fn insert(parent: &[&str], key: &str, value: &str) -> Edit {
    Edit::Insert {
        parent: p(parent),
        key: key.into(),
        value: value.into(),
    }
}

fn remove(path: &[&str]) -> Edit {
    Edit::Remove { path: p(path) }
}

fn run(src: &str, edits: &[Edit]) -> (String, Vec<String>) {
    let out = splice(src, edits).unwrap();
    (out.text, out.lost)
}

#[test]
fn set_replaces_only_the_value_span() {
    let (text, lost) = run(SRC, &[set(&["nodes", "nginx", "proc"], "5ms")]);
    assert_eq!(text, SRC.replace("proc: 1ms", "proc: 5ms"));
    assert_eq!(lost, Vec::<String>::new());
    // a quoted scalar is replaced with its quotes, a nested flow map as a whole
    let q = "a: \"x y\"\nb: { c: 1, d: 2 }\n";
    assert_eq!(run(q, &[set(&["a"], "z")]).0, "a: z\nb: { c: 1, d: 2 }\n");
    assert_eq!(
        run(q, &[set(&["b"], "{ e: 3 }")]).0,
        "a: \"x y\"\nb: { e: 3 }\n"
    );
    // a list item
    let (text, _) = run(SRC, &[set(&["edges", "0", "net"], "9ms")]);
    assert_eq!(text, SRC.replace("net: 1ms", "net: 9ms"));
}

#[test]
fn set_on_an_empty_value_and_on_a_block_collection() {
    assert_eq!(
        run("a:\nb: 1\n", &[set(&["a"], "{ x: 1 }")]).0,
        "a: { x: 1 }\nb: 1\n"
    );
    let block = "m:\n  x: 1\n  y: 2\nn: 3\n";
    assert_eq!(
        run(block, &[set(&["m"], "{ z: 9 }")]).0,
        "m: { z: 9 }\nn: 3\n"
    );
}

#[test]
fn insert_into_a_flow_map_keeps_the_style() {
    let (text, _) = run(SRC, &[insert(&["nodes", "nginx"], "concurrency", "8")]);
    assert_eq!(
        text,
        SRC.replace("proc: 1ms }", "proc: 1ms, concurrency: 8 }")
    );
    // an empty flow map
    assert_eq!(
        run("m: { }\n", &[insert(&["m"], "a", "1")]).0,
        "m: { a: 1 }\n"
    );
    assert_eq!(
        run("m: {}\n", &[insert(&["m"], "a", "1")]).0,
        "m: { a: 1 }\n"
    );
}

#[test]
fn insert_into_a_block_map_matches_the_indent() {
    let (text, _) = run(SRC, &[insert(&["nodes"], "cache", "{ type: cache }")]);
    let want = SRC.replace(
        "  db:    { type: database, ops: { read: 1ms } }\n",
        "  db:    { type: database, ops: { read: 1ms } }\n  cache: { type: cache }\n",
    );
    assert_eq!(text, want);
    // at the root, after the last entry (and before the trailing comment)
    let (text, _) = run(SRC, &[insert(&[], "seed", "42")]);
    assert_eq!(
        text,
        SRC.replace("net: 1ms }\n# end", "net: 1ms }\nseed: 42\n# end")
    );
    // a map that is the first thing on a `- ` line takes its indent from its key's column
    let seq = "edges:\n  - from: a\n    to: b\nz: 1\n";
    let (text, _) = run(seq, &[insert(&["edges", "0"], "net", "1ms")]);
    assert_eq!(text, "edges:\n  - from: a\n    to: b\n    net: 1ms\nz: 1\n");
}

#[test]
fn insert_into_an_empty_document_and_without_a_final_newline() {
    assert_eq!(run("", &[insert(&[], "version", "1")]).0, "version: 1\n");
    assert_eq!(
        run("# nothing yet\n", &[insert(&[], "version", "1")]).0,
        "# nothing yet\nversion: 1\n"
    );
    assert_eq!(run("a: 1", &[insert(&[], "b", "2")]).0, "a: 1\nb: 2");
}

#[test]
fn comments_and_blank_lines_survive_an_edit() {
    let (text, _) = run(
        SRC,
        &[
            set(&["version"], "2"),
            set(&["nodes", "db", "type"], "cache"),
            insert(&["nodes"], "x", "{ type: service }"),
        ],
    );
    for kept in [
        "# the front door and its database\n",
        "   # entry\n",
        "\nedges:\n",
        "# end of file\n",
    ] {
        assert!(text.contains(kept), "lost {kept:?} in:\n{text}");
    }
    assert_eq!(
        text.lines().count(),
        SRC.lines().count() + 1,
        "only the inserted line is new"
    );
}

#[test]
fn crlf_and_bom_are_kept() {
    let src = format!("\u{feff}{}", SRC.replace('\n', "\r\n"));
    let (text, lost) = run(
        &src,
        &[
            set(&["nodes", "nginx", "proc"], "5ms"),
            insert(&["nodes"], "cache", "{ type: cache }"),
            insert(&[], "seed", "42"),
        ],
    );
    assert_eq!(lost, Vec::<String>::new());
    assert!(text.starts_with('\u{feff}'), "the BOM stays");
    assert!(
        !text.replace("\r\n", "").contains('\n'),
        "every new line ending is CRLF: {text:?}"
    );
    assert!(text.contains("proc: 5ms") && text.contains("  cache: { type: cache }\r\n"));
    assert!(text.contains("seed: 42\r\n"));
}

#[test]
fn non_ascii_text_before_an_edit_does_not_shift_it() {
    let src = "# é – ü 日本語\nname: \"é q\"\nn: 1\n";
    assert_eq!(
        run(src, &[set(&["n"], "2")]).0,
        "# é – ü 日本語\nname: \"é q\"\nn: 2\n"
    );
    assert_eq!(
        run(src, &[set(&["name"], "x")]).0,
        "# é – ü 日本語\nname: x\nn: 1\n"
    );
    assert_eq!(
        run(src, &[insert(&[], "z", "3")]).0,
        "# é – ü 日本語\nname: \"é q\"\nn: 1\nz: 3\n"
    );
}

#[test]
fn an_anchor_or_alias_target_is_refused_with_a_lost_entry() {
    let src = "x: &a 5\ny: *a\nz: 1\n";
    for target in ["x", "y"] {
        let (text, lost) = run(src, &[set(&[target], "9")]);
        assert_eq!(text, src, "{target} is left alone");
        assert_eq!(lost.len(), 1, "{lost:?}");
        assert!(lost[0].contains(target), "{lost:?}");
    }
    let (text, lost) = run(src, &[remove(&["x"])]);
    assert_eq!((text.as_str(), lost.len()), (src, 1));
    // the rest of the document is still editable
    assert_eq!(run(src, &[set(&["z"], "2")]).0, "x: &a 5\ny: *a\nz: 2\n");
}

#[test]
fn remove_takes_the_whole_entry_and_reports_the_comment_left_behind() {
    // a block entry goes with its line
    let (text, lost) = run(SRC, &[remove(&["nodes", "db"])]);
    assert_eq!(
        text,
        SRC.replace("  db:    { type: database, ops: { read: 1ms } }\n", "")
    );
    assert_eq!(lost, Vec::<String>::new());
    // a comment directly above the removed entry stays and is reported
    let c = "nodes:\n  # the cache, keep it small\n  cache: { type: cache }\n  db: { type: database }\n";
    let (text, lost) = run(c, &[remove(&["nodes", "cache"])]);
    assert_eq!(
        text,
        "nodes:\n  # the cache, keep it small\n  db: { type: database }\n"
    );
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert!(lost[0].contains("# the cache, keep it small"), "{lost:?}");
    // flow map entries: first, middle, last, only
    let m = "m: { a: 1, b: 2, c: 3 }\n";
    assert_eq!(run(m, &[remove(&["m", "a"])]).0, "m: { b: 2, c: 3 }\n");
    assert_eq!(run(m, &[remove(&["m", "b"])]).0, "m: { a: 1, c: 3 }\n");
    assert_eq!(run(m, &[remove(&["m", "c"])]).0, "m: { a: 1, b: 2 }\n");
    assert_eq!(run("m: { a: 1 }\n", &[remove(&["m", "a"])]).0, "m: {}\n");
    // a block sequence item and a flow sequence item
    assert_eq!(
        run("l:\n  - a\n  - b\n  - c\n", &[remove(&["l", "1"])]).0,
        "l:\n  - a\n  - c\n"
    );
    assert_eq!(
        run("l: [a, b, c]\n", &[remove(&["l", "1"])]).0,
        "l: [a, c]\n"
    );
    assert_eq!(
        run("l: [a, b, c]\n", &[remove(&["l", "2"])]).0,
        "l: [a, b]\n"
    );
}

#[test]
fn a_missing_path_or_a_duplicate_key_is_an_error_and_edits_apply_in_order() {
    let missing = splice(SRC, &[set(&["nodes", "nope", "x"], "1")]);
    assert!(
        matches!(missing, Err(SpliceError::NotFound(_))),
        "{missing:?}"
    );
    let dup = splice(SRC, &[insert(&["nodes"], "db", "{ type: cache }")]);
    assert!(matches!(dup, Err(SpliceError::Exists(_))), "{dup:?}");
    let bad = splice("a: [1, 2\n", &[set(&["a"], "1")]);
    assert!(matches!(bad, Err(SpliceError::Parse(_))), "{bad:?}");
    // a later edit sees the earlier one
    let (text, _) = run(
        SRC,
        &[
            insert(&["nodes"], "cache", "{ type: cache }"),
            set(&["nodes", "cache", "type"], "service"),
        ],
    );
    assert!(text.contains("  cache: { type: service }\n"), "{text}");
}

#[test]
fn unedited_bytes_never_change() {
    // setting every scalar to the text it already has must reproduce the file byte for byte
    for src in [
        SRC,
        include_str!("../../kaanal-server/tests/fixtures/kaanal.yaml"),
        "a: \"q\"\nb: 'r'\nc: [1, 2]\nd:\n  - x: 1\n    y: 2\n# t\n",
    ] {
        let leaves = kaanal_core::splice::scalar_paths(src).unwrap();
        assert_ne!(leaves.len(), 0);
        let edits: Vec<Edit> = leaves
            .into_iter()
            .map(|(path, text)| Edit::Set { path, value: text })
            .collect();
        let out = splice(src, &edits).unwrap();
        assert_eq!(out.text, src);
        assert_eq!(out.lost, Vec::<String>::new());
    }
}

// ---- found by the review of part 2c: each of these once lost or corrupted text --------------

#[test]
fn removing_a_block_scalar_or_an_empty_value_does_not_take_the_next_entry() {
    let src = "a: |\n  hi\n  there\nb: 1\n";
    assert_eq!(run(src, &[remove(&["a"])]).0, "b: 1\n");
    let src = "a:\nb: 1\n";
    let (text, lost) = run(src, &[remove(&["a"])]);
    assert_eq!(text, "b: 1\n");
    assert_eq!(lost, Vec::<String>::new());
    let src = "x:\n  a: >\n    folded\n  c: 2\nb: 1\n";
    assert_eq!(run(src, &[remove(&["x", "a"])]).0, "x:\n  c: 2\nb: 1\n");
    // CRLF
    let src = "a: |\r\n  hi\r\nb: 1\r\n";
    assert_eq!(run(src, &[remove(&["a"])]).0, "b: 1\r\n");
}

#[test]
fn inserting_after_a_block_scalar_or_empty_last_value_stays_in_its_parent() {
    let src = "p:\n  a: |\n    hi\nb: 1\n";
    assert_eq!(
        run(src, &[insert(&["p"], "k", "1")]).0,
        "p:\n  a: |\n    hi\n  k: 1\nb: 1\n"
    );
    let src = "p:\n  a:\nb: 1\n";
    assert_eq!(
        run(src, &[insert(&["p"], "k", "1")]).0,
        "p:\n  a:\n  k: 1\nb: 1\n"
    );
}

#[test]
fn setting_a_block_scalar_keeps_the_next_key_on_its_own_line() {
    let src = "a: >\n  hi\nb: 1\n";
    assert_eq!(run(src, &[set(&["a"], "2")]).0, "a: 2\nb: 1\n");
    let src = "a: |-\n  one\n  two\n\nb: 1\n";
    let text = run(src, &[set(&["a"], "2")]).0;
    assert!(
        text.ends_with("b: 1\n") && text.starts_with("a: 2"),
        "{text:?}"
    );
    assert_eq!(
        serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text).unwrap()["b"],
        1
    );
}

#[test]
fn editing_inside_an_anchored_value_is_refused_not_applied() {
    let src = "base: &b { l: 1ms }\no: *b\n";
    let (text, lost) = run(src, &[set(&["base", "l"], "9ms")]);
    assert_eq!(text, src);
    assert_eq!(lost.len(), 1, "{lost:?}");
    let (text, lost) = run(src, &[insert(&["base"], "k", "1")]);
    assert_eq!((text.as_str(), lost.len()), (src, 1));
    let block = "base: &b\n  l: 1ms\n  m: 2ms\no: *b\n";
    let (text, lost) = run(block, &[remove(&["base", "l"])]);
    assert_eq!((text.as_str(), lost.len()), (block, 1));
    // a sibling of the anchored value is still editable
    assert_eq!(
        run(src, &[set(&["o"], "x")]).1.len(),
        1,
        "an alias is not edited either"
    );
    assert_eq!(
        run("a: &x 1\nc: 2\n", &[set(&["c"], "3")]).0,
        "a: &x 1\nc: 3\n"
    );
}

#[test]
fn quoted_keys_are_found_by_their_value() {
    let src = "\"a\": 1\n'b c': 2\n";
    assert_eq!(run(src, &[set(&["a"], "9")]).0, "\"a\": 9\n'b c': 2\n");
    assert_eq!(run(src, &[remove(&["b c"])]).0, "\"a\": 1\n");
    assert!(matches!(
        splice(src, &[insert(&[], "b c", "1")]),
        Err(SpliceError::Exists(_))
    ));
}

#[test]
fn an_inserted_key_can_never_carry_syntax() {
    let (text, _) = run("a: 1\n", &[insert(&[], "x: 1\nevil", "2")]);
    let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
    assert_eq!(v.as_mapping().unwrap().len(), 2, "{text:?}");
    assert_eq!(v["x: 1\nevil"], 2);
    // odd but harmless keys are quoted too, bare ones stay bare
    let (text, _) = run(
        "a: 1\n",
        &[
            insert(&[], "yes", "1"),
            insert(&[], "404", "2"),
            insert(&[], "ok_key-2", "3"),
        ],
    );
    assert_eq!(text, "a: 1\n\"yes\": 1\n\"404\": 2\nok_key-2: 3\n");
    let (text, _) = run("m: { a: 1 }\n", &[insert(&["m"], "b c", "2")]);
    assert_eq!(text, "m: { a: 1, \"b c\": 2 }\n");
}

#[test]
fn duplicate_keys_are_a_parse_error_not_a_silent_first_match() {
    let r = splice("a: 1\na: 2\n", &[set(&["a"], "3")]);
    assert!(matches!(r, Err(SpliceError::Parse(m)) if m.contains("duplicate key 'a'")));
}

#[test]
fn what_an_edit_cannot_keep_is_listed() {
    // a comment between the key and a block value goes with the value
    let (text, lost) = run("a: # about a\n  b: 1\nc: 2\n", &[set(&["a"], "{ x: 1 }")]);
    assert_eq!(text, "a: { x: 1 }\nc: 2\n");
    assert_eq!(lost.len(), 1, "{lost:?}");
    // removing the only child leaves its parent empty, and says so
    let (text, lost) = run("a:\n  b: 1\nc: 2\n", &[remove(&["a", "b"])]);
    assert_eq!(text, "a:\nc: 2\n");
    assert!(lost.iter().any(|l| l.contains("only entry")), "{lost:?}");
    // a root edit says it is the document, not a list
    let (_, lost) = run("a: 1\n", &[set(&[], "x")]);
    assert!(lost[0].contains("whole document"), "{lost:?}");
}

#[test]
fn a_key_is_renamed_in_place_and_nothing_else_moves() {
    let src = "nodes:\n  # the store\n  db:    { type: database }   # primary\n  app: { type: service }\n";
    let rename = |path: &[&str], to: &str| Edit::RenameKey {
        path: p(path),
        to: to.into(),
    };
    let (text, lost) = run(src, &[rename(&["nodes", "db"], "store")]);
    assert_eq!(text, src.replace("db:", "store:"));
    assert_eq!(lost, Vec::<String>::new());
    // a quoted key, and a new name that needs quotes
    let (text, _) = run("\"a b\": 1\nc: 2\n", &[rename(&["a b"], "x y")]);
    assert_eq!(text, "\"x y\": 1\nc: 2\n");
    // a name that is taken, a missing key, a list item and a key inside an anchored map
    assert!(matches!(
        splice(src, &[rename(&["nodes", "db"], "app")]),
        Err(SpliceError::Exists(_))
    ));
    assert!(matches!(
        splice(src, &[rename(&["nodes", "zz"], "q")]),
        Err(SpliceError::NotFound(_))
    ));
    let (text, lost) = run("l: [a, b]\n", &[rename(&["l", "0"], "q")]);
    assert_eq!((text.as_str(), lost.len()), ("l: [a, b]\n", 1));
    let anchored = "base: &b { l: 1 }\no: *b\n";
    let (text, lost) = run(anchored, &[rename(&["base", "l"], "m")]);
    assert_eq!((text.as_str(), lost.len()), (anchored, 1));
}

#[test]
fn deeply_nested_text_is_refused_not_walked() {
    let deep = "- ".repeat(5000)
        + "x
";
    let err = splice(
        &deep,
        &[Edit::Set {
            path: p(&["0"]),
            value: "1".into(),
        }],
    )
    .unwrap_err();
    assert!(err.to_string().contains("nested deeper"), "{err}");
}
