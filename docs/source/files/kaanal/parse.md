# `crates/kaanal/src/util/parse.rs`

Small argument parsers and text cleaners shared by the commands (durations, rates, path segments, control-text cleaning). Split out of `main.rs` (R3.8); the overview of the commands is in [main.md](main.md).

## Functions

| Function            | What it does                                                                                                                                |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `clean(s)`          | Replaces control characters with `?` — server text can echo user input (a node name) and must never drive the terminal.                     |
| `encode_segment(s)` | Percent-encodes one URL path segment via `percent-encoding` (RFC 3986 unreserved stay, set `SEGMENT`). Used for node names in control URLs. |
| `parse_ms(s)`       | `300ms` / `2s` / `1m 30s` → milliseconds via `humantime`. A unit is required; overflow is an error.                                         |
| `parse_rate(s)`     | `10%` or `0.1` → 0.0–1.0.                                                                                                                   |
