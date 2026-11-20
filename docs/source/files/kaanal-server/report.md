# `crates/kaanal-server/src/sim/report.rs`

The sweep report (Step 9b): one finished sweep record turned into a Markdown document that ends in a fenced JSON block holding the same data. A person reads it cold; an agent parses the JSON block. The server is the only maker of these bytes: the report route, the CLI and Studio hand them over unchanged (no second renderer, no wording shared with the CLI except through the JSON).

## What it does

`sweep_report(rec, version)` reads one `RunRecord` (see [sim.md](sim.md)) and returns `SweepReport { markdown, json }`. `version` is the `kaanal` version the recipe names (`env!("CARGO_PKG_VERSION")` at the call site). It refuses, never panics:

| Record                                                      | Error                     |
| ----------------------------------------------------------- | ------------------------- |
| `kind` is `run`                                             | `NotASweep`               |
| `state` is `running`                                        | `Running`                 |
| `state` is `failed`                                         | `Failed(error)`           |
| done, but no `sweep`, `seed`, `max_factor` or `fingerprint` | `Internal(field)` (a bug) |

**The JSON** (`report_version` 1; keys sorted, as `serde_json`'s map keeps them): `label`, `id`, `finding` (`verdict`, `limit { p99_ms, error_rate }`, `first_to_saturate`, `dominant_cause`, `recovers_at`, `stopped_at`, `warnings`), `points` (from `sweep.points`, sorted; per-node data left out, so the report names only the nodes its finding mentions) and `recipe` (`command`, `seed`, `max_factor`, `profile`, `inline_profile`, `scenario`, `fingerprint`, `generation`, `kaanal_version`).

**The Markdown**, in order: `# Load sweep <id>`; the label in italics; `## Finding` (the verdict in the CLI's words: `knee at N req/s (Kx): ...` with what crossed, `over the limit already at the lowest point, ...`, `holds to ...` or, for a cut ladder (`stopped_at`), `held to ..., the last point run (the ladder stopped above it)`; then the limit, the first node to saturate with its analytic figure, the dominant cause and the warnings); `## Curve` (`load req/s | goodput | p99 ms | errors | ok`, `ok` naming which limit a point is over); `## Re-run` (a `bash` block: the command, then comments for the seed, the fingerprint and what a match means, and the version); `## Data` (the JSON, `serde_json::to_string_pretty`, with format characters written as `\u` escapes).

**The command.** `kaanal sim sweep --profile=NAME --p99 <humantime> --errors <pct>% --max-factor M --seed S [--scenario=NAME]`; the `=` form keeps a name that starts with `-` a value. An inline profile (`inline_profile` set; a config profile may itself be named `inline`) gives `-f profile.yaml`, and the profile is in `recipe.inline_profile` (JSON is YAML, so it can be saved as that file). `--errors` prints the share x 100 to nine decimals with trailing zeros cut, so `0.07` gives `7%`, not `7.000000000000001%`.

**Hostile names.** Node, profile and scenario names come from the user's files. `clean` drops control characters and Unicode format characters (General_Category Cf: bidi overrides and isolates, zero-width spaces; the Trojan Source attack, CVE-2021-42574). In the prose, `text` cleans a name, backslash-escapes the backslash, the backtick and `* _ [ ] < > | ~ &` (CommonMark 0.31 section 2.4; `|` and `~` for GFM tables and strikethrough), and escapes a leading block marker (`#`, `+`, `-`, `=`, or 1 to 9 digits then `.` or `)`), so a warning cannot open a heading, list or quote inside its list item. In the command a name is cleaned and POSIX single-quoted (XCU 2.2.2) unless it is all `[A-Za-z0-9._-]`, so no line in the `bash` fence can start with a fence. In the JSON fence names go through `serde_json`, which escapes every control character (RFC 8259), and `json_text` writes format characters as `\u` escapes (it still parses back to the same value); a pretty-printed line never starts with a backtick, so it cannot close the fence (CommonMark section 4.5). The table holds only numbers and fixed words.

## Items

| Item                                               | Kind            | What it does                                                                                                                                                      |
| -------------------------------------------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sweep_report(rec, version)`                       | fn (crate)      | The report of a finished sweep record                                                                                                                             |
| `SweepReport`                                      | struct (crate)  | `markdown` and `json`                                                                                                                                             |
| `ReportError`                                      | enum (crate)    | `NotASweep`, `Running`, `Failed(String)`, `Internal(&'static str)`                                                                                                |
| `REPORT_VERSION`                                   | const (private) | `1`; bump when a field changes meaning or goes away                                                                                                               |
| `finding`, `curve`                                 | fn (private)    | The `## Finding` and `## Curve` sections                                                                                                                          |
| `command`                                          | fn (private)    | The re-run command                                                                                                                                                |
| `limit_pct`, `row_pct`                             | fn (private)    | A limit as `--errors` reads it back (a limit under 1e-9 % keeps its digits, never `0%`); a point's error rate, two decimals, `<0.01%`                             |
| `clean`, `is_format`, `text`, `shell`, `json_text` | fn (private)    | Drop control and format characters; the Cf table (Unicode 15.1); escape for Markdown text; quote as one shell word; the JSON block with format characters escaped |

Until the report route (Step 9b, task 5) calls it, `sim/mod.rs` declares the module with `cfg_attr(not(test), expect(dead_code))`; the route removes that.

## Tests

Unit tests in the file. `the_report_matches_the_golden_file` pins the bytes against `tests/golden/sweep-report.md` (a three-point knee record; read with `include_str!`; excluded from `dprint` in `dprint.json` so a formatter never rewrites it) and checks the JSON block equals `to_string_pretty` of the returned `json`. Others: the JSON block parses back to `json`; refusals (`Running`, `Failed` keeping the message, `NotASweep`, `Internal` for each missing field); the label is the first line after the heading and a JSON field; an inline profile gives `-f profile.yaml` and is embedded; a config profile named `inline` stays `--profile=inline`; a name starting with `-` stays a value (`--profile=-x`); a warning starting with a block marker is escaped; format characters are stripped; a tiny limit is not `0%`; the limit formats (`500ms`, `1s`, `'1s 500ms'`, `2%`, `0.5%`, `7%`); a cut ladder says `held to`, not `holds to`; a node, profile and scenario holding a backtick, a pipe, a newline, a fence, a bell and a bidi override keep the table's columns, the fences closed, the JSON parsable and no control or format character in the text.
