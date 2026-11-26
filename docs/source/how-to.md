# How-to guide

Practical recipes for working on Kaanal. New here? Read [../coding-guide/index.md](../coding-guide/index.md) first.

## Build, run, test

```bash
cargo build --workspace          # build everything
cargo test --workspace           # must pass before any task is done (project rule R7.1)
cargo run -p kaanal -- validate -c path/to/kaanal.yaml
cargo run -p kaanal -- up -c path/to/kaanal.yaml --port 8080
```

A ready-made playground config lives in `crates/kaanal-server/tests/fixtures/` (`kaanal.yaml` + `openapi.yaml`).

## Add a CLI subcommand

1. Add a variant to `Cmd` in `crates/kaanal/src/main.rs` (clap derive does the parsing).
2. Handle it in `run()`. If it talks to the running instance, use `call(method, path, body)` — it finds the host/port/token from `.kaanal/run.json` (checking the instance lock first) and maps a missing or stale instance to exit 2, and a running one that does not answer to exit 1. Arguments you can check yourself are rejected before the request with `Fail::usage` (exit 64); a value only the server can refuse stays exit 1.
3. Remember: the CLI is a **thin client** of the control API (rule R2.6). No logic in the bin that the server could own.
4. Add an integration test in the matching module of `crates/kaanal/tests/cli/` (shared helpers such as `Up` are in `common.rs`) (spawns a real `kaanal up` on an OS-chosen port).

## Add a control API endpoint

1. Add the route in `router()` in `crates/kaanal-server/src/lib.rs`, **above** the `route_layer` line so the `guard` middleware covers it — the guard is what enforces Host and token checks.
2. Write the handler in the matching file in `crates/kaanal-server/src/control/` (re-export it in `mod.rs`). Take `State<Arc<AppState>>`; return `Json<Value>` or the `Reject` tuple for errors.
3. If it mutates anything, it must need the token — the guard does this for you unless the path is `health` or `ui` (do not extend that exemption list without a security discussion).
4. Test it in the matching module of `crates/kaanal-server/tests/http/` with the `Srv` helper from `common.rs`.

## Add a node type

1. Add the kind to `NodeKind` in `crates/kaanal-core/src/config/nodes.rs` (lowercase serde rename).
2. Extend validation in `Topology::build` (`core/src/topology.rs`): update `callees` (the edge table) and add the kind to `NodeConfig`; the schema test fails until every new field has a doc comment. Anything the simulator cannot honour must be **rejected loudly**, not silently ignored.
3. Give it a cost in the chain executor (`core/src/exec.rs`). Rule R2.5: timing/failure logic lives **once** in core; never re-implement it in the server or (later) the simulator.
4. Tests: topology validation in `core/tests/topology.rs`, chain behaviour in `core/tests/exec.rs`.

## Add a step kind

1. Add the visitor in `crates/kaanal-core/src/config/endpoints.rs` and its schema in `schema.rs` (every new field needs a doc comment; the schema test fails otherwise).
2. Resolve it in `chain.rs`: compile it into a step, and count it against the limits.
3. Walk it in `exec.rs`, with a new draw tag so existing seeds keep their numbers.
4. The simulator needs nothing unless the step adds a new `PlanItem`.
5. Tests: `core/tests/chain.rs` (compile), `core/tests/semantics.rs` (timeline) and the drift tests in `kaanal-sim`.

## Change the cost model (latency, errors)

Everything lives in `crates/kaanal-core/src/dist.rs` (distributions) and `exec.rs` (the walk). Keep these invariants, they are tested:

- Same `(seed, endpoint_key, n)` → same outcome, and editing one step never changes another step's draws (each hop has its own seed).
- The chaos error roll is always drawn, so chaos settings never shift timing RNG.
- `execute()` stays pure — no sleeping, no IO. The caller sleeps.

## Change generated data

Generator: `crates/kaanal-data/src/generate.rs`. An `allOf` is merged into one schema first (`merge_all_of`); then the priority order is `example → examples → const → enum → oneOf/anyOf → type (project dictionary → format → built-in dictionary → constraints)`.

- Field names map to generators in `crates/kaanal-data/data/dictionary.yaml` (a closed generator set in `dictionary.rs`); names, places, phones and currency come from the locale packs in `data/locales/` (`pack.rs`). Edit those files, not Rust, to change what a field looks like; `words.rs` only keeps the lorem words.
- If you add support for a schema keyword that was previously ignored, remove it from `IGNORED` in `warn.rs` — the warning list and the generator must stay in sync.
- Rule R4.4: every field derives its seed from `(parent_seed, name, index)` via `child_seed`. Never thread one RNG through sibling fields.
- Rule R4.5: formats beat lengths — never truncate/pad a formatted value.
- Self-check: `data/tests/generate.rs::output_validates_against_schema` validates generated output with `jsonschema`. Keep it passing.

## Add a static route override

Users do this in `kaanal.yaml` under `endpoints:` (`"GET /health": { override: { body: { ok: true } } }`). The merge/validation logic is in `AppState::build` (`server/src/state.rs`): shape conflicts (`/users/{id}` vs `/users/{x}`), reserved `/__kaanal` namespace, method and status checks. Tests in `server/tests/state.rs`.

## Cut a release

Releases are manual (R1.60, RULES 148).

1. Set the version in `Cargo.toml` (`[workspace.package]`) and write `RELEASE.md`: the first line is `Tag: vX.Y.Z`, which must equal that version, and the rest is the release notes. `bash scripts/release-info.sh` checks both locally.
2. Merge it to `main`.
3. Run the **Release** workflow on `main` with "publish" ticked. It builds Linux x86_64, macOS arm64 and Windows x86_64 archives (macOS x86_64 is skipped), dry-runs the crates.io and then the npm publish, and only if both pass creates the tag and a GitHub release (a pre-release below 1.0) with the archives and `SHA256SUMS`. Each archive holds the `kaanal` binary, `LICENSE`, `THIRD_PARTY_LICENSES.md` and `README.md`.
4. The same run then publishes the six crates to crates.io (`crates` job) and the npm packages (`npm` job), by trusted publishing: no registry token is stored in the repository, each registry accepts the run's GitHub identity. The `npm` job builds one package per platform plus the `kaanal` wrapper with `scripts/npm-pack.mjs` (checked by `node --test scripts/npm-pack.test.mjs`), and `kaanal-server` ships Studio's bundle inside the crate, which the `crates` job checks first. A failed registry job can be re-run from the Actions page.
5. **Approve the staged npm versions.** The `npm` job runs `npm stage publish` (npm 11.15.0 or newer), so nothing is installable from npm until you approve it with 2FA: the Staged Packages tab on npmjs.com, or `npm stage approve`. Approve the three `@kaanal/*` packages first, then `kaanal`.
6. Verify as a user: `cargo install kaanal` and `npx kaanal --version`.
7. To try it first, run the workflow with "publish" off: it builds the archives (kept as artifacts for a week) and does the dry runs, tagging nothing.

**The dry runs.** Every run does them before the tag: `cargo publish --workspace --dry-run --locked --allow-dirty` (`--allow-dirty` because the Studio bundle is git-ignored but packaged, and cargo counts it as uncommitted), then `npm stage publish --dry-run` on each npm package. With "publish" on, a version already on crates.io or npm fails the run. With it off, npm cannot dry-run a published version, so it only lists the package contents (`npm pack --dry-run`). They use no token, so a wrong trusted-publisher entry still shows only at publish time.

**A tag can be used once.** With GitHub immutable releases on, a tag name stays taken even after the release and tag are deleted. A published registry version is permanent too. So if a run fails after something went out, re-run the failed job; if that cannot work, bump the version.

**One-time setup for trusted publishing** (the header of `release.yml` has the exact fields).

- GitHub: environments `crates-io` and `npm`.
- crates.io has no "pending publisher": publish each of the six crates once by hand with an API token (`cargo publish --workspace` after `cd ui && pnpm build`), then add the trusted publisher to each crate: repository `Naveen2070/kaanal`, workflow `release.yml`, environment `crates-io`.
- npm is the same: publish `kaanal`, `@kaanal/linux-x64`, `@kaanal/darwin-arm64` and `@kaanal/win32-x64` once by hand (the `@kaanal` org must exist), then add the trusted publisher to each package: workflow `release.yml`, environment `npm`, allowed action stage publish (the workflow only stages). npm does not check the entry when you save it.
- The first hand-published version cannot also be the CI one, so either publish a placeholder (`0.0.0`) of each first and let the workflow publish `0.1.0`, or publish `0.1.0` by hand and use the workflow from `0.1.1`.

**What the binaries are.** `Cargo.toml` has no `[profile.release]` section, so they are default release builds: not stripped, no LTO, no size tuning. They are larger than they could be. A size and speed tuning pass (`strip`, `lto`, `codegen-units`, `panic = "abort"`) is a separate job and needs measuring first (`scripts/bench.sh` for speed, the archive sizes for size).

**To do before a public release.**

- **The docs site must answer before you tag.** The crates.io and npm metadata, the README and several error messages point at <https://kaanal.naveenr.in>; a published version cannot be fixed. Check `/docs/start/install/` and `/docs/guides/simulate/traffic/` load (the Vercel secrets, domain and DNS are in "Docs site" below), then run the Release workflow.
- Tune the release profile (above) once it has been measured.
- Pin the GitHub Actions to commit SHAs, with Dependabot to keep them current.
- Optional: signed checksums or build attestations; Linux musl and arm64 builds; macOS x86_64.
- Install channels (Homebrew, Scoop, winget, Docker) are M4.

## Third-party licenses

`THIRD_PARTY_LICENSES.md` (repo root, committed, generated) holds the notices for what ships in a release: the Rust crates of the `kaanal` binary (`cargo-about`, with `about.toml` listing the accepted licences and `about.hbs` the layout), Studio's production JavaScript libraries (`pnpm licenses`), the three fonts (`licenses/fonts/`, the OFL texts from the pinned Fontsource packages) and a note on the price data. It goes in every archive and every npm package.

- Regenerate after any change to `Cargo.lock` or `ui/pnpm-lock.yaml`: `cargo install cargo-about --locked --features cli` once, `cd ui && pnpm install`, then `node scripts/third-party-licenses.mjs`. It takes a few minutes. A licence cargo-about does not accept fails the run: read it, then add it to `about.toml`.
- `node scripts/third-party-licenses.mjs --check` compares the lock-file hashes in the file's first line with the current ones (no tools needed); the Release workflow runs it first and stops a stale file.
- A new font needs its OFL text in `licenses/fonts/` as well as a row in `ui/FONTS.md`.
- `dprint` skips the file and `licenses/`: the texts must stay verbatim.

## Docs site

The user docs are a Starlight site in `site/`, published at <https://kaanal.naveenr.in> (`docs/web/` no longer exists).

- **Edit a page:** the Markdown is in `site/src/content/docs/docs/`; see `site/README.md`. `pnpm build` in `site/` fails on a broken internal link or anchor, so run it before you push.
- **Deploy:** `.github/workflows/site.yml` builds on every site change; a push to `main` deploys to production and a pull request gets a preview URL (in the job summary). The deploy job skips itself, with a notice, when the secrets are missing (a fork pull request, or before setup).
- **One-time setup (the maintainer):**
  1. In Vercel, create a project from this repository. Leave **Root Directory empty** (the workflow runs the Vercel commands from `site/`), framework Astro, and turn off Vercel's own Git deploys so only the workflow deploys.
  2. Add three repository secrets: `VERCEL_TOKEN` (an account token), `VERCEL_ORG_ID` and `VERCEL_PROJECT_ID` (both in `.vercel/project.json` after `npx vercel link` in `site/`, or in the project settings).
  3. Add the domain `kaanal.naveenr.in` to the Vercel project and create the DNS record Vercel shows (a CNAME for the `kaanal` subdomain).
- The Vercel Hobby plan is for non-commercial use; check the plan if Kaanal is ever sold or sponsored.

## Debugging tips

- Every mock response carries `X-Kaanal-Trace` with the per-hop waterfall, e.g. `nginx.proc=1.0ms,net=1.0ms,app.proc=1.0ms,net=1.0ms,db.read=1.0ms`.
- `GET /__kaanal/events` (SSE, needs `?token=`) streams every request as it happens.
- Every mock response also carries `X-Request-Id`. `kaanal trace <id>` shows that request hop by hop (flags, the cause of a failure, queue waits), `kaanal logs` lists the last 1000; both read the in-memory ring in `server/src/log.rs`. To change what a flag means, edit `record.rs::build_entry`/`hop_recs` and its tests, and the table in the Observability guide (`site/src/content/docs/docs/guides/observe/observability.md`).
- On Windows, sleeps under ~15 ms round up to the OS timer tick — measured latency can exceed the simulated sum. That is a known platform limitation, not a bug in your change.
