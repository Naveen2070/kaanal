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
2. Handle it in `run()`. If it talks to the running instance, use `call(method, path, body)` — it finds the host/port/token from `.kaanal/run.json` and maps failures to exit code 2.
3. Remember: the CLI is a **thin client** of the control API (rule R2.6). No logic in the bin that the server could own.
4. Add an integration test in `crates/kaanal/tests/cli.rs` using the `Up` helper (spawns a real `kaanal up` on an OS-chosen port).

## Add a control API endpoint

1. Add the route in `router()` in `crates/kaanal-server/src/lib.rs`, **above** the `route_layer` line so the `guard` middleware covers it — the guard is what enforces Host and token checks.
2. Write the handler in `crates/kaanal-server/src/control.rs`. Take `State<Arc<AppState>>`; return `Json<Value>` or the `Reject` tuple for errors.
3. If it mutates anything, it must need the token — the guard does this for you unless the path is `health` or `ui` (do not extend that exemption list without a security discussion).
4. Test it in `crates/kaanal-server/tests/http.rs` with the `Srv` helper.

## Add a node type

1. Add the kind to `NodeKind` in `crates/kaanal-core/src/config.rs` (lowercase serde rename).
2. Extend validation in `Topology::build` (`core/src/topology.rs`): update `callees` (the edge table) and add the kind to `NodeConfig`; the schema test fails until every new field has a doc comment. Anything the simulator cannot honour must be **rejected loudly**, not silently ignored.
3. Give it a cost in the chain executor (`core/src/exec.rs`). Rule R2.5: timing/failure logic lives **once** in core; never re-implement it in the server or (later) the simulator.
4. Tests: topology validation in `core/tests/topology.rs`, chain behaviour in `core/tests/exec.rs`.

## Change the cost model (latency, errors)

Everything lives in `crates/kaanal-core/src/dist.rs` (distributions) and `exec.rs` (the walk). Keep these invariants, they are tested:

- Same `(seed, endpoint_key, n)` → same outcome.
- The chaos error roll is always drawn, so chaos settings never shift timing RNG.
- `execute()` stays pure — no sleeping, no IO. The caller sleeps.

## Change generated data

Generator: `crates/kaanal-data/src/generate.rs`. An `allOf` is merged into one schema first (`merge_all_of`); then the priority order is `example → enum → oneOf/anyOf → type (format → field-name heuristic → constraints → fallback)`.

- Field-name heuristics live in `heuristic()`; word lists in `words.rs`.
- If you add support for a schema keyword that was previously ignored, remove it from `IGNORED` in `warn.rs` — the warning list and the generator must stay in sync.
- Rule R4.4: every field derives its seed from `(parent_seed, name, index)` via `child_seed`. Never thread one RNG through sibling fields.
- Rule R4.5: formats beat lengths — never truncate/pad a formatted value.
- Self-check: `data/tests/generate.rs::output_validates_against_schema` validates generated output with `jsonschema`. Keep it passing.

## Add a static route override

Users do this in `kaanal.yaml` under `routes:` (`"GET /health": { body: { ok: true } }`). The merge/validation logic is in `AppState::build` (`server/src/state.rs`): shape conflicts (`/users/{id}` vs `/users/{x}`), reserved `/__kaanal` namespace, method and status checks. Tests in `server/tests/state.rs`.

## Debugging tips

- Every mock response carries `X-Kaanal-Trace` with the per-hop waterfall, e.g. `nginx.proc=1.0ms,net=1.0ms,app.proc=1.0ms,net=1.0ms,db.read=1.0ms`.
- `GET /__kaanal/events` (SSE, needs `?token=`) streams every request as it happens.
- On Windows, sleeps under ~15 ms round up to the OS timer tick — measured latency can exceed the simulated sum. That is a known platform limitation, not a bug in your change.
