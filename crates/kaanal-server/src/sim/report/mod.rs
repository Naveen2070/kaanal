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

use super::store::{LiveChaos, RunRecord};

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
    let chaos = chaos_in_force(rec)?;
    let chaos_digest = rec
        .chaos_digest
        .as_deref()
        .ok_or(ReportError::Internal("chaos_digest"))?;
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
            "chaos": chaos.kind(),
            "live_chaos": rec.live_chaos,
            "chaos_digest": chaos_digest,
            "runtime_scenarios": rec.runtime_scenarios,
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
    finding(&mut md, s, &chaos, chaos_digest);
    curve(&mut md, &s.points);
    let _ = write!(md, "## Re-run\n\n```bash\n{command}\n");
    if rec.inline_profile.is_some() {
        md.push_str(
            "# profile.yaml: recipe.inline_profile in the Data block below (JSON is YAML)\n",
        );
    }
    if let Chaos::Live(l) = chaos {
        let _ = writeln!(
            md,
            "# live chaos in force when this ran ({}): the command alone cannot reproduce it; activate the same chaos first",
            live_text(l, clean)
        );
    }
    if !rec.runtime_scenarios.is_empty() {
        let names: Vec<String> = rec.runtime_scenarios.iter().map(|n| clean(n)).collect();
        let _ = writeln!(
            md,
            "# scenarios defined at runtime in force ({}): the command alone cannot reproduce it; define them the same way first and compare the chaos digest",
            names.join(", ")
        );
    }
    let _ = write!(
        md,
        "# seed {seed}: every point draws the same random numbers, so the same seed on the same design repeats the curve\n\
         # fingerprint {}: SHA-256 of the design files the build read, comments and whitespace aside; it does not cover the chaos in force (here: {}; chaos digest {}): the same fingerprint, the same chaos digest and the same seed mean the same numbers, the fingerprint alone does not\n\
         # kaanal {}\n```\n\n## Data\n\n```json\n",
        clean(fingerprint),
        chaos.describe(clean),
        clean(chaos_digest),
        clean(version)
    );
    md.push_str(&json_text(&json));
    md.push_str("\n```\n");
    Ok(SweepReport { markdown: md, json })
}

/// The chaos a sweep ran under, as the report names it.
enum Chaos<'a> {
    /// No scenario named, none active and no live entry.
    None,
    /// `--scenario NAME`: that scenario's chaos alone, which the command repeats when the scenario
    /// is in a file or the config (a runtime one is flagged by `runtime_scenarios`).
    Scenario(&'a str),
    /// The live chaos (active scenarios, live entries); the command cannot repeat its live entries.
    Live(&'a LiveChaos),
}

/// The chaos `rec` ran under; a sweep without a scenario and without its `live_chaos` is a bug.
fn chaos_in_force(rec: &RunRecord) -> Result<Chaos<'_>, ReportError> {
    if let Some(sc) = &rec.scenario {
        return Ok(Chaos::Scenario(sc));
    }
    let l = rec
        .live_chaos
        .as_ref()
        .ok_or(ReportError::Internal("live_chaos"))?;
    Ok(if l.scenarios.is_empty() && l.entries == 0 {
        Chaos::None
    } else {
        Chaos::Live(l)
    })
}

impl Chaos<'_> {
    /// `recipe.chaos` in the JSON.
    fn kind(&self) -> &'static str {
        match self {
            Chaos::None => "none",
            Chaos::Scenario(_) => "scenario",
            Chaos::Live(_) => "live",
        }
    }

    /// In words, each name passed through `name` (`text` in the prose, `clean` in the bash block).
    fn describe(&self, name: fn(&str) -> String) -> String {
        match self {
            Chaos::None => "none".into(),
            Chaos::Scenario(n) => format!("scenario {}", name(n)),
            Chaos::Live(l) => format!("live chaos: {}", live_text(l, name)),
        }
    }
}

/// `active scenarios a, b; 3 entries, digest X` (the scenarios part only when one is active).
fn live_text(l: &LiveChaos, name: fn(&str) -> String) -> String {
    let mut s = String::new();
    if !l.scenarios.is_empty() {
        let names: Vec<String> = l.scenarios.iter().map(|n| name(n)).collect();
        let _ = write!(s, "active scenarios {}; ", names.join(", "));
    }
    let unit = if l.entries == 1 { "entry" } else { "entries" };
    let _ = write!(s, "{} {unit}, digest {}", l.entries, clean(&l.digest));
    s
}

/// `v` pretty-printed, every format character and line or paragraph separator written as a `\u` escape (RFC 8259 section 7 lets any
/// character be escaped, and in this text they occur only inside strings), so a name in the block
/// cannot reorder or hide text either; it parses back to `v` unchanged.
fn json_text(v: &Value) -> String {
    // a Value always serialises
    let pretty = serde_json::to_string_pretty(v).unwrap_or_default();
    let mut out = String::with_capacity(pretty.len());
    for c in pretty.chars() {
        if hidden(c) {
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
/// node to saturate, the dominant cause, the chaos in force with its digest, and the warnings.
fn finding(md: &mut String, s: &SweepResult, chaos: &Chaos<'_>, chaos_digest: &str) {
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
        // the knee is the last point within the limit; `crossed` is the next point's, the first over
        Verdict::Knee { k, req_s } => format!(
            "knee at {}, the last point within the limit; the next point is over{crossed}",
            at(k, req_s)
        ),
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
        "**Verdict:** {verdict}.\n\n- Limit: p99 {p99}, errors {errors}.\n- First to saturate: {first}.\n- Dominant cause: {cause}.\n- Chaos in force: {}.\n- Chaos digest: {}.\n\n",
        chaos.describe(text),
        text(chaos_digest)
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
        .filter(|&c| !c.is_control() && !hidden(c))
        .collect()
}

/// A format character, or U+2028 / U+2029 (`Zl`, `Zp`: line breaks to some renderers and to
/// JavaScript before ES2019). The CLI prints all of them as `?`, so the server drops them from the
/// text and escapes them in the JSON, and its bytes pass the CLI's cleaning unchanged.
fn hidden(c: char) -> bool {
    is_format(c) || matches!(c, '\u{2028}' | '\u{2029}')
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
mod tests;
