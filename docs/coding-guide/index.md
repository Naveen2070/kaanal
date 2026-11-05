# Coding guide — start here

Welcome to Kaanal. This page is the contributor intro: how the code is written, the rules that bite, how commits work, and where to read next depending on what you want to change.

## 1. Coding style

The codebase is small and deliberately plain. Match it:

- **Rust edition 2024, rustc ≥ 1.99** (pinned in the workspace `Cargo.toml`). Never name anything `gen` — it's a reserved keyword.
- **Let the types carry the rules.** Invalid states are rejected at load (`Topology::build`, `AppState::build`), not checked defensively at request time. Handlers trust their inputs.
- **Errors are precise.** Every user-facing error says what was wrong and where: `file:line:col: message` for config, the operation JSON-pointer for OpenAPI, the edge number for topology. Never `unwrap` on user input; never return a bare `Err("invalid")`.
- **No surprises in errors.** Unknown config fields are rejected (`deny_unknown_fields`). Typos fail loudly.
- **Comments explain _why_, not what.** Look at existing files: comments state invariants, rules and 3am failure modes ("the roll is always drawn so the RNG sequence doesn't depend on chaos settings"). Known limitations are marked `// limitation:`.
- **Determinism is a feature.** All randomness flows from a seeded RNG (`HopRng` for per-hop draws in `kaanal-core`, `ChaCha8Rng` for simulator arrivals and generated data) seeded by `stable_hash` (FNV-1a). Never use std hashers, wall-clock time, or environment state in output paths.
- **Comments/doc style:** inline comments only where they earn their place. **Rustdoc (`///`) is required on every item, public or private** — `#![warn(missing_docs)]` is on in the library crates and the hook denies warnings. Public items: say what it does, what it trusts, what can go wrong; public `Result` fns need an `# Errors` section. **Private items start their doc with `**Private.**`** so generated docs stay honest.
- **Lint policy:** `rustfmt.toml` + `[workspace.lints]` in the root `Cargo.toml` pin the style: clippy **pedantic** (with a curated allow-list documented right there), `unsafe_code` forbidden (the one exception is `kaanal-winhandle`, which holds the single Win32 call that keeps `up -d` daemons off the caller's pipes: R1.46), every crate opts in with `[lints] workspace = true`. If a lint fights the codebase's idiom, adjust the allow-list in the root `Cargo.toml` _with a comment why_ — don't sprinkle `#[allow]` in code. For genuinely-local exceptions (e.g. test helpers), use `#[expect(lint, reason = "...")]`.

## 2. Project rules that bite

The rules that shape everyday code. Rule IDs (e.g. `R4.1`) also appear in code comments as stable labels:

| Rule      | Gist                                                                                                                                                                                                                                                                      |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R2.2–R2.4 | Dependencies flow one way: `kaanal` → `kaanal-server` → {`core`, `data`}. Core is pure (no tokio/axum/IO). Data never depends on core. Only server knows HTTP; only the bin knows argv. `kaanal-sim` → `kaanal-core` only; it never computes timing itself (R2.5).        |
| R2.5      | **One cost model, two drivers.** Timing/failure logic lives once in `kaanal-core`; live mode and the simulator (`kaanal-sim`) both call it. New node behaviour lands in core first.                                                                                       |
| R2.6      | Every interface (CLI, web UI, future TUI) is a thin client of the `/__kaanal` control API. No per-interface logic.                                                                                                                                                        |
| R2.7      | Topology is a DAG; invalid graphs are rejected at load.                                                                                                                                                                                                                   |
| R3.3      | YAML is `serde_yaml_ng`. Never `serde_yaml` (deprecated) or `serde_yml` (unsoundness advisory).                                                                                                                                                                           |
| R3.4      | Phase one: no `openapiv3`, no `fake`, no `tower-http`. OpenAPI is parsed into `serde_json::Value`; CORS is set by hand.                                                                                                                                                   |
| R3.8      | Config is **YAML** (read-only, no emitter); machine state, the control API and the JSON Schema are **JSON**; **TOML is never a config format**. Adding a TOML dependency needs a new decision log.                                                                        |
| R4.1–R4.5 | Determinism rules: stable seeds per endpoint/field, chaos roll always drawn, formats beat lengths. See [../source/files/kaanal-core/exec.md](../source/files/kaanal-core/exec.md) and [../source/files/kaanal-data/generate.md](../source/files/kaanal-data/generate.md). |
| R5.x      | Security: loopback bind, token-guarded control API, Host-header check, CORS on mocks only, `.kaanal/` user-private, synthetic data only (`example.com`).                                                                                                                  |
| R6.1–R6.3 | YAML errors `file:line:col: msg`; topology/spec errors name the node, edge or route. Exit 1; never serve half-configured; exit codes 0 ok / 1 error / 2 not running / 3 reserved / 64 usage error (see [../web/lifecycle.md](../web/lifecycle.md#exit-codes)).            |
| R6.7      | One instance per directory via OS file lock.                                                                                                                                                                                                                              |
| R7.1      | `cargo test --workspace` passes before any task is done.                                                                                                                                                                                                                  |
| R1.5      | **Scope guard:** say no by default to Kafka, gRPC, plugins, containers, wire-protocol DB mocks until users ask.                                                                                                                                                           |

## 3. Commits & git

- **Branches:** work on a feature branch, never directly on `main`.
- **Commit only when asked.** Otherwise stage the work and wait.
- **Every commit is signed off** (`git commit -s`, DCO).
- **Message style:** short subject + a compact, human bullet list of what was done. Gist, not a changelog. **No `Co-Authored-By` or other AI attribution trailers** unless explicitly requested by the developer.
- **Identity:** author, committer, and sign-off use the GitHub noreply `104181086+Naveen2070@users.noreply.github.com`.

## 4. "I want to change X" → read these docs

| You want to…                            | Read first                                                                           | Then                                                                                                                                                                              |
| --------------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Understand the whole system             | [../source/architecture.md](../source/architecture.md)                               | [../source/how-to.md](../source/how-to.md)                                                                                                                                        |
| Add/change a CLI command                | [../source/files/kaanal/main.md](../source/files/kaanal/main.md)                     | how-to: _Add a CLI subcommand_                                                                                                                                                    |
| Add a control API endpoint              | [../source/files/kaanal-server/control.md](../source/files/kaanal-server/control.md) | [../source/files/kaanal-server/lib.md](../source/files/kaanal-server/lib.md), how-to: _Add a control API endpoint_                                                                |
| Change mock request handling            | [../source/files/kaanal-server/mock.md](../source/files/kaanal-server/mock.md)       | [../source/files/kaanal-server/state.md](../source/files/kaanal-server/state.md)                                                                                                  |
| Change timing, latency or failure logic | [../source/files/kaanal-core/exec.md](../source/files/kaanal-core/exec.md)           | [../source/files/kaanal-core/dist.md](../source/files/kaanal-core/dist.md), [../source/files/kaanal-core/chaos.md](../source/files/kaanal-core/chaos.md)                          |
| Add a node type or change validation    | [../source/files/kaanal-core/topology.md](../source/files/kaanal-core/topology.md)   | [../source/files/kaanal-core/config.md](../source/files/kaanal-core/config.md), how-to: _Add a node type_                                                                         |
| Change `kaanal.yaml` syntax             | [../source/files/kaanal-core/config.md](../source/files/kaanal-core/config.md)       | topology + state pages (validation lives downstream)                                                                                                                              |
| Change generated mock data              | [../source/files/kaanal-data/generate.md](../source/files/kaanal-data/generate.md)   | [../source/files/kaanal-data/warn.md](../source/files/kaanal-data/warn.md) (keep `IGNORED` in sync), [../source/files/kaanal-data/words.md](../source/files/kaanal-data/words.md) |
| Change OpenAPI parsing                  | [../source/files/kaanal-data/api.md](../source/files/kaanal-data/api.md)             | warn + generate pages                                                                                                                                                             |
| Work on the embedded status page        | [../web/web-ui.md](../web/web-ui.md)                                                 | [../source/files/kaanal-server/control.md](../source/files/kaanal-server/control.md) (CSP, token-from-fragment)                                                                   |
| Add or fix tests                        | [../source/tests.md](../source/tests.md)                                             | the per-file page for the code under test                                                                                                                                         |

## 5. Before you commit

0. **One-time setup after cloning:** `git config core.hooksPath .githooks` — the pre-commit hook checks only what the staged files can affect, so a small change commits quickly: **Rust files** run `cargo fmt --all --check`, then clippy (`-- -D warnings`, all targets) and the tests for the changed crates and every crate that depends on them (a change that only touches one integration-test suite runs just that suite, `cargo test -p <crate> --test <suite>`); **UI files** run `vp check` and the UI tests related to the changed files (all of them when a config, the lockfile or anything outside `ui/src` changed); **Markdown** runs `dprint check`; a commit that touches both Rust and UI runs both, and one that touches neither runs nothing. The plan comes from `scripts/hook-plan.mjs` (it needs node; without it the hook runs everything), and `KAANAL_HOOK_FULL=1 git commit …` runs everything on purpose. The CI always runs everything, so this only keeps the local loop fast. Style and lints are pinned by `rustfmt.toml`, `[workspace.lints]` in `Cargo.toml`, and `dprint.json` in the repo root.
1. `cargo test --workspace` green (the hook checks, but run it yourself first — it's faster than a rejected commit).
2. `cargo fmt --all`, `dprint fmt`, and clippy clean.
3. Determinism intact: if you touched seeding, re-check the invariants in R4.x and the tests that pin them.
4. Security intact: if you touched the control API, re-check R5.x — the guard, CORS split, token handling.
5. On a feature branch, signed off, human message.
