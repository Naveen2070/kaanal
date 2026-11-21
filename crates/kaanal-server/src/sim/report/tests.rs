//! The sweep report's unit tests, kept apart from `report/mod.rs` (AGENTS.md file size).

use std::sync::Arc;
use std::time::Duration;

use kaanal_sim::sweep::{FirstSaturated, Limit, NodePoint, Over, Point, SweepResult, Verdict};

use super::*;
use crate::sim::store::LiveChaos;

const GOLDEN: &str = include_str!("../../../tests/golden/sweep-report.md");
const GOLDEN_LIVE: &str = include_str!("../../../tests/golden/sweep-report-live-chaos.md");

fn point(k: f64, goodput: f64, error_rate: f64, p99_ms: f64, over: Over) -> Point {
    Point {
        k,
        req_s: 100.0 * k,
        requests: if k < 4.0 { 20_000 } else { 900 },
        goodput_req_s: goodput,
        error_rate,
        p50_ms: p99_ms / 4.0,
        p95_ms: p99_ms / 2.0,
        p99_ms,
        cause: (error_rate > 0.01).then_some("overflow"),
        nodes: vec![NodePoint {
            name: "db".into(),
            utilization: Some(k / 4.0),
            saturated: k >= 4.0,
            saturates_at_req_s: Some(400.0),
        }],
        over,
    }
}

/// A finished sweep: three points, a knee at 2x, the db saturated at 4x.
fn rec() -> RunRecord {
    let both = Over {
        p99: true,
        errors: true,
    };
    RunRecord {
        id: "s3".into(),
        kind: "sweep",
        state: "done",
        generation: 2,
        profile: "peak".into(),
        scenario: None,
        source: "applied",
        draft_rev: None,
        started_ms: 1,
        finished_ms: Some(2),
        summary: None,
        cost: None,
        error: None,
        sweep_points: Vec::new(),
        sweep: Some(SweepResult {
            label: "relative what-if, not a prediction",
            limit: Limit::default(),
            points: vec![
                point(1.0, 100.0, 0.0, 20.0, Over::default()),
                point(2.0, 199.5, 0.0025, 250.0, Over::default()),
                point(4.0, 380.0, 0.05, 2500.0, both),
            ],
            verdict: Verdict::Knee {
                k: 2.0,
                req_s: 200.0,
            },
            recovers_at: None,
            stopped_at: None,
            first_to_saturate: Some(FirstSaturated {
                node: "db".into(),
                k: 4.0,
                req_s: 400.0,
                analytic_req_s: Some(400.0),
            }),
            dominant_cause: Some("overflow"),
            warnings: vec!["at 4.000x the p99 rests on 900 requests".into()],
        }),
        seed: Some(42),
        max_factor: Some(8.0),
        fingerprint: Some("0123456789ab".into()),
        inline_profile: None,
        live_chaos: Some(LiveChaos {
            scenarios: Vec::new(),
            entries: 0,
            digest: "e3b0c44298fc".into(),
        }),
        chaos_digest: Some("9f86d081884c".into()),
        runtime_scenarios: Vec::new(),
        log: Arc::default(),
    }
}

/// The text of the first fenced block opened by `fence`.
fn block<'a>(md: &'a str, fence: &str) -> &'a str {
    let start = md.find(fence).expect("the block") + fence.len();
    let end = start + md[start..].find("\n```\n").expect("closed");
    &md[start..end]
}

fn command(r: &RunRecord) -> String {
    let rep = sweep_report(r, "9.9.9").unwrap();
    rep.json["recipe"]["command"].as_str().unwrap().to_string()
}

#[test]
fn the_report_matches_the_golden_file() {
    let rep = sweep_report(&rec(), "9.9.9").unwrap();
    assert_eq!(rep.markdown, GOLDEN);
    assert_eq!(
        block(&rep.markdown, "```json\n"),
        serde_json::to_string_pretty(&rep.json).unwrap()
    );
}

#[test]
fn the_json_block_is_the_returned_json() {
    let rep = sweep_report(&rec(), "9.9.9").unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(block(&rep.markdown, "```json\n")).unwrap();
    assert_eq!(parsed, rep.json);
    assert_eq!(rep.json["report_version"], 1);
    assert_eq!(rep.json["points"].as_array().unwrap().len(), 3);
    assert!(
        rep.json["points"][0].get("nodes").is_none(),
        "per-node data is left out"
    );
    assert_eq!(rep.json["recipe"]["kaanal_version"], "9.9.9");
}

#[test]
fn a_running_failed_or_run_record_is_refused() {
    let running = RunRecord {
        state: "running",
        sweep: None,
        ..rec()
    };
    assert_eq!(
        sweep_report(&running, "v").unwrap_err(),
        ReportError::Running
    );
    let failed = RunRecord {
        state: "failed",
        sweep: None,
        error: Some("boom at 2x".into()),
        ..rec()
    };
    assert_eq!(
        sweep_report(&failed, "v").unwrap_err(),
        ReportError::Failed("boom at 2x".into())
    );
    let run = RunRecord {
        kind: "run",
        ..rec()
    };
    assert_eq!(sweep_report(&run, "v").unwrap_err(), ReportError::NotASweep);
    // a done sweep without the recipe fields is a server bug, refused, not a panic
    for broken in [
        RunRecord {
            seed: None,
            ..rec()
        },
        RunRecord {
            max_factor: None,
            ..rec()
        },
        RunRecord {
            fingerprint: None,
            ..rec()
        },
        RunRecord {
            sweep: None,
            ..rec()
        },
        // a sweep without --scenario always records the live chaos it ran under
        RunRecord {
            live_chaos: None,
            ..rec()
        },
        // every sweep records the digest of its compiled chaos
        RunRecord {
            chaos_digest: None,
            ..rec()
        },
    ] {
        assert!(matches!(
            sweep_report(&broken, "v").unwrap_err(),
            ReportError::Internal(_)
        ));
    }
}

#[test]
fn the_label_is_the_first_content_line_and_a_json_field() {
    let rep = sweep_report(&rec(), "v").unwrap();
    let mut lines = rep.markdown.lines().filter(|l| !l.is_empty());
    assert_eq!(lines.next(), Some("# Load sweep s3"));
    assert_eq!(lines.next(), Some("*relative what-if, not a prediction*"));
    assert_eq!(rep.json["label"], "relative what-if, not a prediction");
}

#[test]
fn an_inline_profile_uses_f_and_is_embedded() {
    let profile = serde_json::json!({ "rps": 50, "journeys": [{ "path": "/a" }] });
    let r = RunRecord {
        profile: "inline".into(),
        inline_profile: Some(profile.clone()),
        ..rec()
    };
    let rep = sweep_report(&r, "v").unwrap();
    let cmd = command(&r);
    assert!(
        cmd.starts_with("kaanal sim sweep -f profile.yaml --p99"),
        "{cmd}"
    );
    assert!(!cmd.contains("--profile"), "{cmd}");
    assert_eq!(rep.json["recipe"]["inline_profile"], profile);
    assert!(block(&rep.markdown, "```bash\n").contains("profile.yaml"));
    // a named profile has none
    assert!(sweep_report(&rec(), "v").unwrap().json["recipe"]["inline_profile"].is_null());
}

#[test]
fn a_config_profile_named_inline_is_a_named_profile() {
    // `traffic: { inline: ... }` is a valid profile name: only `inline_profile` means posted inline
    let r = RunRecord {
        profile: "inline".into(),
        ..rec()
    };
    let rep = sweep_report(&r, "v").unwrap();
    let cmd = command(&r);
    assert!(
        cmd.starts_with("kaanal sim sweep --profile=inline --p99"),
        "{cmd}"
    );
    assert!(!rep.markdown.contains("profile.yaml"), "{}", rep.markdown);
    assert!(rep.json["recipe"]["inline_profile"].is_null());
}

#[test]
fn a_name_starting_with_a_dash_stays_a_value() {
    let r = RunRecord {
        profile: "-x".into(),
        scenario: Some("--help".into()),
        ..rec()
    };
    let cmd = command(&r);
    assert!(cmd.contains(" --profile=-x "), "{cmd}");
    assert!(cmd.ends_with(" --scenario=--help"), "{cmd}");
}

#[test]
fn a_warning_cannot_open_a_block() {
    let mut r = rec();
    r.sweep.as_mut().unwrap().warnings = [
        "# head", "1. one", "12) two", "+ plus", "- minus", "> quote", "= under",
    ]
    .map(String::from)
    .to_vec();
    let md = sweep_report(&r, "v").unwrap().markdown;
    for want in [
        r"- \# head",
        r"- 1\. one",
        r"- 12\) two",
        r"- \+ plus",
        r"- \- minus",
        r"- \> quote",
        r"- \= under",
    ] {
        assert!(md.lines().any(|l| l == want), "{want} in {md}");
    }
    // mid-text punctuation is left alone
    assert_eq!(text("a-b 1. c"), "a-b 1. c");
}

#[test]
fn format_characters_are_stripped() {
    // Unicode General_Category Cf: soft hyphen, zero-width space, bidi override and isolate, BOM, a tag
    assert_eq!(
        clean("a\u{AD}b\u{200B}c\u{202E}d\u{2066}e\u{FEFF}f\u{E0041}g"),
        "abcdefg"
    );
    assert_eq!(
        clean("caf\u{E9} \u{1F600}"),
        "caf\u{E9} \u{1F600}",
        "letters and emoji stay"
    );
}

#[test]
fn a_tiny_limit_is_not_zero_percent() {
    assert_eq!(limit_pct(0.0), "0%");
    assert_eq!(limit_pct(1e-12), "0.0000000001%");
    assert_eq!(limit_pct(0.01), "1%");
}

#[test]
fn the_command_formats_the_limits() {
    let with = |p99: Duration, error_rate: f64| {
        let mut r = rec();
        r.sweep.as_mut().unwrap().limit = Limit { p99, error_rate };
        command(&r)
    };
    assert_eq!(
        command(&rec()),
        "kaanal sim sweep --profile=peak --p99 1s --errors 1% --max-factor 8 --seed 42"
    );
    let c = with(Duration::from_millis(500), 0.02);
    assert!(c.contains("--p99 500ms --errors 2% "), "{c}");
    let c = with(Duration::from_secs(1), 0.005);
    assert!(c.contains("--p99 1s --errors 0.5% "), "{c}");
    // float noise in x100 (0.07 * 100 = 7.000000000000001) does not leak into the flag
    let c = with(Duration::from_millis(1500), 0.07);
    assert!(c.contains("--p99 '1s 500ms' --errors 7% "), "{c}");
    let mut r = rec();
    r.max_factor = Some(2.5);
    r.scenario = Some("db-down".into());
    assert!(command(&r).ends_with("--max-factor 2.5 --seed 42 --scenario=db-down"));
}

#[test]
fn a_cut_ladder_says_held_to_not_holds_to() {
    let mut r = rec();
    let s = r.sweep.as_mut().unwrap();
    s.verdict = Verdict::HoldsTo {
        k: 2.0,
        req_s: 200.0,
    };
    s.points.truncate(2);
    s.stopped_at = Some(4.0);
    let md = sweep_report(&r, "v").unwrap().markdown;
    assert!(
        md.contains("held to 200 req/s (2.00x), the last point run (the ladder stopped above it)"),
        "{md}"
    );
    assert!(!md.contains("holds to"));
    r.sweep.as_mut().unwrap().stopped_at = None;
    let md = sweep_report(&r, "v").unwrap().markdown;
    assert!(
        md.contains("holds to 200 req/s (2.00x): no point over p99 1s or errors 1%"),
        "{md}"
    );
    let mut r = rec();
    r.sweep.as_mut().unwrap().verdict = Verdict::BrokenAtLowest {
        k: 0.25,
        req_s: 25.0,
    };
    let md = sweep_report(&r, "v").unwrap().markdown;
    assert!(
        md.contains("over the limit already at the lowest point, 25 req/s (0.25x): "),
        "{md}"
    );
}

#[test]
fn hostile_names_cannot_break_the_markdown_or_the_json() {
    let hostile = "x`|y\n```\u{7}\u{202E}z";
    let mut r = rec();
    r.scenario = Some(hostile.into());
    r.profile = "my peak|`\r\n#".into();
    r.sweep
        .as_mut()
        .unwrap()
        .first_to_saturate
        .as_mut()
        .unwrap()
        .node = hostile.into();
    let rep = sweep_report(&r, "v").unwrap();
    let md = &rep.markdown;
    assert!(md.chars().all(|c| c == '\n' || !c.is_control()), "{md:?}");
    assert!(!md.contains('\u{202E}'), "no bidi override: {md:?}");
    // every table line has the header's six unescaped pipes
    let pipes = |l: &str| {
        l.match_indices('|')
            .filter(|(i, _)| !l[..*i].ends_with('\\'))
            .count()
    };
    for l in md.lines().filter(|l| l.starts_with('|')) {
        assert_eq!(pipes(l), 6, "{l}");
    }
    // fences: four lines, each block closed, the JSON still parses with the names intact
    assert_eq!(
        md.lines()
            .filter(|l| l.trim_start().starts_with("```"))
            .count(),
        4,
        "{md}"
    );
    let parsed: serde_json::Value = serde_json::from_str(block(md, "```json\n")).unwrap();
    assert_eq!(parsed["recipe"]["scenario"], hostile);
    assert!(
        block(md, "```json\n").contains(r"\u202e"),
        "the override is escaped"
    );
    assert_eq!(parsed, rep.json);
    // the command quotes the names, control characters gone
    let cmd = block(md, "```bash\n").lines().next().unwrap();
    assert!(cmd.contains("--profile='my peak|`#'"), "{cmd}");
    assert!(cmd.ends_with("--scenario='x`|y```z'"), "{cmd}");
    // the node name in the prose is escaped
    assert!(md.contains(r"First to saturate: x\`\|y\`\`\`z at"), "{md}");
}

/// A sweep that ran under live chaos: two active scenarios and three live entries.
fn live() -> RunRecord {
    RunRecord {
        live_chaos: Some(LiveChaos {
            scenarios: vec!["slow-db".into(), "flaky".into()],
            entries: 3,
            digest: "abcdef012345".into(),
        }),
        ..rec()
    }
}

#[test]
fn the_knee_is_the_last_point_within_the_limit_and_the_next_is_over() {
    // Verdict::Knee is the last passing point (200 req/s); the crossing named is the next one's
    let md = sweep_report(&rec(), "v").unwrap().markdown;
    assert!(
        md.contains("**Verdict:** knee at 200 req/s (2.00x), the last point within the limit; the next point is over: p99 over 1s and errors over 1%.\n"),
        "{md}"
    );
}

#[test]
fn no_chaos_says_none_and_the_fingerprint_covers_the_files_only() {
    let rep = sweep_report(&rec(), "v").unwrap();
    let md = &rep.markdown;
    assert!(md.contains("- Chaos in force: none.\n"), "{md}");
    let bash = block(md, "```bash\n");
    assert!(
        bash.contains("it does not cover the chaos in force (here: none; chaos digest 9f86d081884c): the same fingerprint, the same chaos digest and the same seed mean the same numbers, the fingerprint alone does not"),
        "{bash}"
    );
    assert!(!bash.contains("live chaos in force"), "{bash}");
    assert_eq!(rep.json["recipe"]["chaos"], "none");
    assert_eq!(rep.json["recipe"]["live_chaos"]["entries"], 0);
}

#[test]
fn a_scenario_sweep_names_the_scenario_and_its_command_repeats_it() {
    let r = RunRecord {
        scenario: Some("db-down".into()),
        live_chaos: None,
        ..rec()
    };
    let rep = sweep_report(&r, "v").unwrap();
    let md = &rep.markdown;
    assert!(md.contains("- Chaos in force: scenario db-down.\n"), "{md}");
    let bash = block(md, "```bash\n");
    assert!(
        bash.lines()
            .next()
            .unwrap()
            .ends_with(" --scenario=db-down"),
        "{bash}"
    );
    assert!(
        bash.contains("(here: scenario db-down; chaos digest 9f86d081884c)"),
        "{bash}"
    );
    assert!(!bash.contains("live chaos in force"), "{bash}");
    assert_eq!(rep.json["recipe"]["chaos"], "scenario");
    assert!(rep.json["recipe"]["live_chaos"].is_null());
    assert_eq!(rep.json["recipe"]["scenario"], "db-down");
}

#[test]
fn live_chaos_is_named_and_the_recipe_says_the_command_alone_cannot_repeat_it() {
    let rep = sweep_report(&live(), "v").unwrap();
    let md = &rep.markdown;
    let what = "active scenarios slow-db, flaky; 3 entries, digest abcdef012345";
    assert!(
        md.contains(&format!("- Chaos in force: live chaos: {what}.\n")),
        "{md}"
    );
    let bash = block(md, "```bash\n");
    assert!(
        bash.contains(&format!("# live chaos in force when this ran ({what}): the command alone cannot reproduce it; activate the same chaos first\n")),
        "{bash}"
    );
    assert!(
        bash.contains(&format!(
            "(here: live chaos: {what}; chaos digest 9f86d081884c)"
        )),
        "{bash}"
    );
    assert_eq!(rep.json["recipe"]["chaos"], "live");
    assert_eq!(rep.json["recipe"]["live_chaos"]["digest"], "abcdef012345");
    assert_eq!(rep.json["recipe"]["live_chaos"]["entries"], 3);
    assert_eq!(rep.json["recipe"]["live_chaos"]["scenarios"][1], "flaky");
    // live entries alone, no scenario active: one entry is singular
    let r = RunRecord {
        live_chaos: Some(LiveChaos {
            scenarios: Vec::new(),
            entries: 1,
            digest: "0a0a0a0a0a0a".into(),
        }),
        ..rec()
    };
    let md = sweep_report(&r, "v").unwrap().markdown;
    assert!(
        md.contains("- Chaos in force: live chaos: 1 entry, digest 0a0a0a0a0a0a.\n"),
        "{md}"
    );
}

#[test]
fn the_live_chaos_report_matches_its_golden_file() {
    let rep = sweep_report(&live(), "9.9.9").unwrap();
    assert_eq!(rep.markdown, GOLDEN_LIVE);
}

#[test]
fn a_line_separator_in_a_name_reads_as_the_cli_would_print_it() {
    // U+2028 and U+2029 are line breaks to some renderers (Zl, Zp: neither control nor format), and
    // the CLI's clean prints them as '?'. The server drops them from the text and escapes them in the
    // JSON, so its bytes pass the CLI's cleaning unchanged: `sim report`, the route and Studio agree.
    let mut r = rec();
    r.profile = "pe\u{2028}ak".into();
    r.scenario = Some("db\u{2029}down".into());
    r.live_chaos = None;
    r.sweep
        .as_mut()
        .unwrap()
        .first_to_saturate
        .as_mut()
        .unwrap()
        .node = "d\u{2028}b".into();
    let rep = sweep_report(&r, "v").unwrap();
    // the CLI's `clean` (crates/kaanal/src/util/parse.rs) replaces these, so none may be left
    let cli_changes = |c: char| {
        c != '\n'
            && (c.is_control()
                || matches!(c, '\u{61c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
    };
    assert!(!rep.markdown.chars().any(cli_changes), "{:?}", rep.markdown);
    let data = block(&rep.markdown, "```json\n");
    assert!(
        data.contains(r"\u2028") && data.contains(r"\u2029"),
        "{data}"
    );
    let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(parsed, rep.json, "the escapes parse back to the names");
    assert!(
        rep.markdown.contains("First to saturate: db at"),
        "{}",
        rep.markdown
    );
}

#[test]
fn every_report_names_the_chaos_digest() {
    for r in [
        rec(),
        live(),
        RunRecord {
            scenario: Some("db-down".into()),
            live_chaos: None,
            ..rec()
        },
    ] {
        let rep = sweep_report(&r, "v").unwrap();
        assert!(
            rep.markdown.contains("- Chaos digest: 9f86d081884c.\n"),
            "{}",
            rep.markdown
        );
        assert_eq!(rep.json["recipe"]["chaos_digest"], "9f86d081884c");
        assert_eq!(
            rep.json["recipe"]["runtime_scenarios"],
            serde_json::json!([])
        );
    }
}

#[test]
fn a_runtime_scenario_in_force_says_the_command_alone_cannot_repeat_it() {
    let r = RunRecord {
        scenario: Some("rt".into()),
        live_chaos: None,
        runtime_scenarios: vec!["rt".into()],
        ..rec()
    };
    let rep = sweep_report(&r, "v").unwrap();
    let bash = block(&rep.markdown, "```bash\n");
    assert!(
        bash.lines().next().unwrap().ends_with(" --scenario=rt"),
        "the flag stays: {bash}"
    );
    assert!(
        bash.contains("# scenarios defined at runtime in force (rt): the command alone cannot reproduce it; define them the same way first and compare the chaos digest\n"),
        "{bash}"
    );
    assert_eq!(
        rep.json["recipe"]["runtime_scenarios"],
        serde_json::json!(["rt"])
    );
    // a file or config scenario has no such line
    let md = sweep_report(
        &RunRecord {
            runtime_scenarios: Vec::new(),
            ..r
        },
        "v",
    )
    .unwrap()
    .markdown;
    assert!(!md.contains("defined at runtime"), "{md}");
}
