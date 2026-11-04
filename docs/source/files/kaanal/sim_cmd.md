# `kaanal/src/commands/sim.rs`

`kaanal sim run|list|show`, `kaanal sample` and `kaanal skeleton`: thin clients of the simulation routes (R2.6; `docs/web/simulation.md`, `docs/source/files/kaanal-server/sim.md`).

- **`sim run`.** `--profile NAME` or `--file FILE` (YAML or JSON, one profile; not both). With neither, it sends the empty name and the server says what the project has (`no traffic profile is defined: ...` or `name one with -p: ...`, RULES 58); the 404 is turned back into a usage error, exit 64, as before, with the server's text alone. `--duration` and `--warmup` are humantime strings sent as seconds (`bad --duration` is a usage error). It posts, then polls `GET /sim/runs/{id}` every 250 ms for up to five minutes (`--no-wait` prints the id and returns). A failed run prints its error and exits 1; a 409 while another run is going carries the server's "already in progress" message.
- **Report text.** The label first, then requests, failures and p50/p95/p99/max, failures by kind, the five busiest nodes (load, utilization, mean wait, measured against expected visits per second, `SATURATED`) and the warnings. `--json` prints the record as one line.
- **`sample ROUTE`.** The positional is named `route` because `--endpoint` is a global flag. The body goes to stdout (pretty JSON), the size and status to stderr, so the body can be piped.
- **`skeleton`.** Prints the YAML; with `-o` the file is opened with `create_new`, so an existing file is refused without `--force` (no check-then-write race).

Tests: `tests/cli/sim.rs`.
