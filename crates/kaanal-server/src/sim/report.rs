//! The sweep report (Step 9b): one finished sweep record as a Markdown file with a fenced JSON
//! block holding the same data, so a person can read it cold and an agent can parse it. The
//! server is the only maker; the route, the CLI and Studio hand over these bytes unchanged.
//!
//! Names (node, profile, scenario) come from the user's files, so none can break the document:
//! outside a fence they lose their control characters and have Markdown's punctuation
//! backslash-escaped (`CommonMark` 0.31, section 2.4, "Backslash escapes"); inside the JSON fence they
//! go through `serde_json` (RFC 8259 escapes every control character, and a pretty-printed line
//! never starts with a backtick, so it cannot close the fence, `CommonMark` section 4.5); in the
//! command they are POSIX single-quoted (XCU 2.2.2) after their control characters are dropped, so
//! no line inside the `bash` fence can start with a fence either.

use std::fmt::Write as _;

use kaanal_sim::sweep::{Point, SweepResult, Verdict};
use serde_json::{Value, json};

use super::store::RunRecord;

/// The `report_version` of the JSON: bump it when a field changes meaning or goes away.
const REPORT_VERSION: u32 = 1;

/// A finished sweep's report.
#[derive(Debug)]
pub(crate) struct SweepReport {
    /// The whole document, ending in the `## Data` block that holds [`Self::json`] pretty-printed.
    pub markdown: String,
    /// The data alone (keys sorted, as `serde_json`'s map keeps them).
    pub json: Value,
}

/// Why a record has no report.
#[derive(Debug, PartialEq)]
pub(crate) enum ReportError {
    /// A plain run: its report stays in Studio.
    NotASweep,
    /// The sweep has not finished.
    Running,
    /// The sweep failed; why.
    Failed(String),
    /// A finished sweep without the fields every sweep record gets (a server bug); which one.
    Internal(&'static str),
}

/// The report of the finished sweep `rec`; `version` is the `kaanal` version the recipe names.
///
/// # Errors
/// [`ReportError`] for a run, a running or failed sweep, or a sweep record missing its recipe.
pub(crate) fn sweep_report(rec: &RunRecord, version: &str) -> Result<SweepReport, ReportError> {
    if rec.kind != "sweep" {
        return Err(ReportError::NotASweep);
    }
    match rec.state {
        "running" => return Err(ReportError::Running),
        "failed" => return Err(ReportError::Failed(rec.error.clone().unwrap_or_default())),
        _ => {}
    }
    let s = rec.sweep.as_ref().ok_or(ReportError::Internal("sweep"))?;
    let seed = rec.seed.ok_or(ReportError::Internal("seed"))?;
    let max_factor = rec.max_factor.ok_or(ReportError::Internal("max_factor"))?;
    let fingerprint = rec
        .fingerprint
        .as_deref()
        .ok_or(ReportError::Internal("fingerprint"))?;
    let command = command(rec, s, max_factor, seed);
    let json = json!({
        "report_version": REPORT_VERSION,
        "label": s.label,
        "id": rec.id,
        "finding": {
            "verdict": s.verdict,
            "limit": s.limit,
            "first_to_saturate": s.first_to_saturate,
            "dominant_cause": s.dominant_cause,
            "recovers_at": s.recovers_at,
            "stopped_at": s.stopped_at,
            "warnings": s.warnings,
        },
        "points": s.points.iter().map(point_json).collect::<Vec<_>>(),
        "recipe": {
            "command": command,
            "seed": seed,
            "max_factor": max_factor,
            "profile": rec.profile,
            "inline_profile": rec.inline_profile,
            "scenario": rec.scenario,
            "fingerprint": fingerprint,
            "generation": rec.generation,
            "kaanal_version": version,
        },
    });
    let mut md = String::new();
    // `write!` into a String cannot fail: the results are dropped
    let _ = write!(
        md,
        "# Load sweep {}\n\n*{}*\n\n## Finding\n\n",
        text(&rec.id),
        text(s.label)
    );
    finding(&mut md, s);
    curve(&mut md, &s.points);
    let _ = write!(md, "## Re-run\n\n```bash\n{command}\n");
    if rec.inline_profile.is_some() {
        md.push_str(
            "# profile.yaml: recipe.inline_profile in the Data block below (JSON is YAML)\n",
        );
    }
    let _ = write!(
        md,
        "# seed {seed}: every point draws the same random numbers, so the same seed on the same design repeats the curve\n\
         # fingerprint {}: SHA-256 of every file the build read, comments and whitespace aside; the same value means the same design, so the same numbers\n\
         # kaanal {}\n```\n\n## Data\n\n```json\n",
        clean(fingerprint),
        clean(version)
    );
    md.push_str(&json_text(&json));
    md.push_str("\n```\n");
    Ok(SweepReport { markdown: md, json })
}

/// `v` pretty-printed, every format character written as a `\u` escape (RFC 8259 section 7 lets any
/// character be escaped, and in this text they occur only inside strings), so a name in the block
/// cannot reorder or hide text either; it parses back to `v` unchanged.
fn json_text(v: &Value) -> String {
    // a Value always serialises
    let pretty = serde_json::to_string_pretty(v).unwrap_or_default();
    let mut out = String::with_capacity(pretty.len());
    for c in pretty.chars() {
        if is_format(c) {
            for u in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(out, "\\u{u:04x}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One point without its per-node data (the report names only the nodes its finding mentions).
fn point_json(p: &Point) -> Value {
    json!({
        "k": p.k,
        "req_s": p.req_s,
        "requests": p.requests,
        "goodput_req_s": p.goodput_req_s,
        "error_rate": p.error_rate,
        "p50_ms": p.p50_ms,
        "p95_ms": p.p95_ms,
        "p99_ms": p.p99_ms,
        "cause": p.cause,
        "over": p.over,
    })
}

/// The `## Finding` section: the verdict in one sentence (the CLI's wording), the limit, the first
/// node to saturate, the dominant cause and the warnings.
fn finding(md: &mut String, s: &SweepResult) {
    let p99 = humantime::format_duration(s.limit.p99).to_string();
    let errors = limit_pct(s.limit.error_rate);
    let crossed = s
        .points
        .iter()
        .find(|p| p.over.p99 || p.over.errors)
        .map_or(String::new(), |p| {
            let mut why = Vec::new();
            if p.over.p99 {
                why.push(format!("p99 over {p99}"));
            }
            if p.over.errors {
                why.push(format!("errors over {errors}"));
            }
            format!(": {}", why.join(" and "))
        });
    let at = |k: f64, req_s: f64| format!("{req_s:.0} req/s ({k:.2}x)");
    let none_over = format!("no point over p99 {p99} or errors {errors}");
    let verdict = match s.verdict {
        Verdict::Knee { k, req_s } => format!("knee at {}{crossed}", at(k, req_s)),
        Verdict::BrokenAtLowest { k, req_s } => {
            format!(
                "over the limit already at the lowest point, {}{crossed}",
                at(k, req_s)
            )
        }
        // a cut ladder only held to the last point that ran: it does not claim the top
        Verdict::HoldsTo { k, req_s } if s.stopped_at.is_some() => format!(
            "held to {}, the last point run (the ladder stopped above it): {none_over}",
            at(k, req_s)
        ),
        Verdict::HoldsTo { k, req_s } => format!("holds to {}: {none_over}", at(k, req_s)),
    };
    let first = s.first_to_saturate.as_ref().map_or("none".into(), |f| {
        let analytic = f
            .analytic_req_s
            .map_or(String::new(), |a| format!(", analytic {a:.0} req/s"));
        format!("{} at {}{analytic}", text(&f.node), at(f.k, f.req_s))
    });
    let cause = s.dominant_cause.map_or("none".into(), text);
    let _ = write!(
        md,
        "**Verdict:** {verdict}.\n\n- Limit: p99 {p99}, errors {errors}.\n- First to saturate: {first}.\n- Dominant cause: {cause}.\n\n"
    );
    if s.warnings.is_empty() {
        md.push_str("Warnings: none.\n\n");
    } else {
        md.push_str("Warnings:\n\n");
        for w in &s.warnings {
            let _ = writeln!(md, "- {}", text(w));
        }
        md.push('\n');
    }
}

/// The `## Curve` table, the CLI's columns. Every cell is a number or a fixed word, so no cell
/// needs escaping.
fn curve(md: &mut String, points: &[Point]) {
    md.push_str("## Curve\n\n| load req/s | goodput | p99 ms | errors | ok |\n| ---: | ---: | ---: | ---: | --- |\n");
    for p in points {
        let ok = match (p.over.p99, p.over.errors) {
            (false, false) => "yes",
            (true, false) => "no (p99)",
            (false, true) => "no (errors)",
            (true, true) => "no (p99, errors)",
        };
        let _ = writeln!(
            md,
            "| {:.1} | {:.1} | {:.1} | {} | {ok} |",
            p.req_s,
            p.goodput_req_s,
            p.p99_ms,
            row_pct(p.error_rate)
        );
    }
    md.push('\n');
}

/// The command that repeats the sweep: `-f profile.yaml` for an inline profile (posted, `inline_profile`
/// set), else `--profile=NAME`. Names use the `=` form, so one starting with `-` stays a value.
fn command(rec: &RunRecord, s: &SweepResult, max_factor: f64, seed: u64) -> String {
    let profile = if rec.inline_profile.is_some() {
        "-f profile.yaml".to_string()
    } else {
        format!("--profile={}", shell(&rec.profile))
    };
    let p99 = shell(&humantime::format_duration(s.limit.p99).to_string());
    let errors = limit_pct(s.limit.error_rate);
    let mut c = format!(
        "kaanal sim sweep {profile} --p99 {p99} --errors {errors} --max-factor {max_factor} --seed {seed}"
    );
    if let Some(sc) = &rec.scenario {
        let _ = write!(c, " --scenario={}", shell(sc));
    }
    c
}

/// A limit's share as the percentage `--errors` reads back: `1%`, `0.5%`. Nine decimals hide the
/// float noise of x100 (0.07 x 100 = 7.000000000000001) and keep any limit a person types.
fn limit_pct(share: f64) -> String {
    let p = share * 100.0;
    let s = format!("{p:.9}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    // a limit under 1e-9 % keeps its digits (Display never uses an exponent) rather than reading 0%
    if s == "0" && p != 0.0 {
        format!("{p}%")
    } else {
        format!("{s}%")
    }
}

/// A point's error rate, two decimals as the CLI; `<0.01%` for a share too small for them, so a row
/// never says `0%` and over a limit of zero at once.
fn row_pct(share: f64) -> String {
    let p = share * 100.0;
    if p > 0.0 && p < 0.005 {
        return "<0.01%".into();
    }
    let s = format!("{p:.2}");
    format!("{}%", s.trim_end_matches('0').trim_end_matches('.'))
}

/// `s` without control characters (a newline would end a line, an escape would reach a terminal)
/// or format characters (bidi overrides and isolates, zero-width spaces: they can reorder or hide
/// text, the "Trojan Source" attack, CVE-2021-42574).
fn clean(s: &str) -> String {
    s.chars()
        .filter(|&c| !c.is_control() && !is_format(c))
        .collect()
}

/// True for a character of Unicode `General_Category` Cf (Unicode 15.1, `DerivedGeneralCategory.txt`);
/// the standard library has no category lookup and the list is short and stable.
fn is_format(c: char) -> bool {
    matches!(
        u32::from(c),
        0xAD | 0x600..=0x605
            | 0x61C
            | 0x6DD
            | 0x70F
            | 0x890..=0x891
            | 0x8E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// `s` safe as Markdown text: cleaned, the punctuation that opens code, emphasis, a link, a table
/// cell, HTML or an entity backslash-escaped (`CommonMark` 0.31, section 2.4; `|` and `~` for GFM),
/// and a leading block marker escaped, so text that starts a line (a warning's list item) cannot
/// open a heading, a list item, a block quote or a setext underline (sections 4.2, 4.3, 5.1, 5.2).
fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in clean(s).chars() {
        if "\\`*_[]<>|~&".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    // `>` is escaped above; an ordered list marker is 1 to 9 digits then `.` or `)`
    let digits = out.bytes().take_while(u8::is_ascii_digit).count();
    if out.starts_with(['#', '+', '-', '=']) {
        out.insert(0, '\\');
    } else if (1..=9).contains(&digits) && out[digits..].starts_with(['.', ')']) {
        out.insert(digits, '\\');
    }
    out
}

/// `s` as one shell word: as is when it is all `[A-Za-z0-9._-]`, else cleaned and single-quoted,
/// a quote inside written `'\''` (POSIX XCU 2.2.2: nothing is special between single quotes).
fn shell(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", clean(s).replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use kaanal_sim::sweep::{FirstSaturated, Limit, NodePoint, Over, Point, SweepResult, Verdict};

    use super::*;

    const GOLDEN: &str = include_str!("../../tests/golden/sweep-report.md");

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
            md.contains(
                "held to 200 req/s (2.00x), the last point run (the ladder stopped above it)"
            ),
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
}
