# `crates/kaanal-server/src/generation.rs`

`Generation`: everything one config builds, the part of the server a later Apply can swap.

## What it does

`Generation::build(cfg, base_dir, drawn_seed, lenient)` is the full load pipeline shared by `serve` and `check` (`kaanal validate`): topology → OpenAPI specs → endpoints (inline map or the endpoints file) → scenarios → route table. It returns the generation and the `ScenarioSet` the config and `scenarios/` define, resolved against this topology. Moved out of `state.rs` unchanged (R3.8); `AppState` holds the result behind a lock and hands it out one `Arc` at a time (Studio step 2, part 2a), so a later Apply can swap it.

`drawn_seed` is the `determinism: realistic` startup draw, passed in rather than drawn here so it stays the same across generations. `chain_seed` is `drawn_seed` if given, else `seed:`.

## Types and functions

| Item | Kind | What it does |
| --- | --- | --- |
| `Generation` | struct | `id` (1 for the first generation; each Apply will count up), `topo`, `paths` (matchit router of `Vec<Route>`), `route_keys` (sorted, synthesized HEAD routes excluded), `endpoints`, `counters`, `scenarios_dir`, `site`, `warnings`, `seed`, `chain_seed`. The per-node data `options` and `project_options` are private, read through `options_for_owner`. Immutable once built. |
| `Flagged` | struct | A scenario file the build left out: `file`, `scenario` (its stem) and `message`. |
| `Generation.flagged` | field | The flagged files; always empty for a strict build. |
| `Generation::build(cfg, base_dir, drawn_seed, lenient)` | fn | The load pipeline above. With `lenient`, a scenario file that does not resolve against the new topology is flagged and left out instead of failing (Apply and validate); inline scenarios stay strict, and startup and `check` pass `false`. Any problem is a `Located` error; nothing is served half-configured (R6.1). |
| `Generation.auth`, `Generation.engine` | fields | The auth node's token issuer and the live replay engine handle: `None` after `build` (no runtime and no key in `check`), set by `serve` through `AppState::gen_mut` before the state is shared. |
| `Generation::options_for_owner(owner)` | fn | The generation `Options` of a route's owner node (its `locale:`, else the project's, else `en-IN`); a route with no owner uses the project's. |
| `Gen` | struct, extractor | `Gen(Arc<Generation>)`: the axum extractor a handler uses to take the generation once at its start. Answers 409 `{"error":"no system applied yet; apply a draft"}` while a blank start has none. |
| `spec_routes(cfg, base_dir)` | fn | The topology and its spec routes, for `kaanal scenario` and `init`. |
| `read_api`, `load_data` | fns | Read one node's spec; load the data files and per-node options. Also used by `selfcheck`. |

The private helpers (`check_shape`, `load_specs`, `load_site`, `compile_endpoints`, `load_scenarios`, `data_located`) moved with it and keep their messages.
