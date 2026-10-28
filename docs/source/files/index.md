# Per-file documentation

One page per source file: what the file does and what each function/type in it does. Grouped by crate.

## [kaanal-core](kaanal-core/) (pure logic)

| File                      | Page                                                     | Contents                                                                                                                    |
| ------------------------- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`              | [kaanal-core/lib.md](kaanal-core/lib.md)                 | Crate root, re-exports, `stable_hash`                                                                                       |
| `src/config/mod.rs`       | [kaanal-core/config.md](kaanal-core/config.md)           | `kaanal.yaml` parsing: tagged node kinds, `DistConfig`, `ConfigError`                                                       |
| `src/config/nodes.rs`     | [kaanal-core/config.md](kaanal-core/config.md)           | The node kinds and the per-kind structs (gateway, service, monolith, database, cache, external API, auth), the flavor enums |
| `src/config/limits.rs`    | [kaanal-core/config.md](kaanal-core/config.md)           | Concurrency, pools and rate limits                                                                                          |
| `src/config/shared.rs`    | [kaanal-core/config.md](kaanal-core/config.md)           | Users, edges, latency distributions (`DistConfig`) and endpoint overrides                                                   |
| `src/config/endpoints.rs` | [kaanal-core/config.md](kaanal-core/config.md)           | The `endpoints:` map, error maps and the step deserializers (`call`, `cache`, `parallel`, `repeat`)                         |
| `src/config/scenario.rs`  | [kaanal-core/config.md](kaanal-core/config.md)           | Scenario files: `chaos:` entries, `pins:`, `parse_scenario`                                                                 |
| `src/config/values.rs`    | [kaanal-core/config.md](kaanal-core/config.md)           | Field-level serde helpers (ratios, rates, statuses, URLs, durations, unique names)                                          |
| `src/config/checks.rs`    | [kaanal-core/config.md](kaanal-core/config.md)           | Cross-field checks serde cannot place: auth, capacity, locales                                                              |
| `src/diagnostics.rs`      | [kaanal-core/diagnostics.md](kaanal-core/diagnostics.md) | Error positions: saphyr tree, per-branch schema check, `Located`                                                            |
| `src/presets.rs`          | [kaanal-core/presets.md](kaanal-core/presets.md)         | Flavor presets (estimates) and gap filling                                                                                  |
| `src/schema.rs`           | [kaanal-core/schema.md](kaanal-core/schema.md)           | JSON Schema of `kaanal.yaml` (schemars)                                                                                     |
| `src/topology.rs`         | [kaanal-core/topology.md](kaanal-core/topology.md)       | Graph build + validation (optional gateway, edge table, DAG, presets)                                                       |
| `src/replay/`             | [kaanal-core/replay.md](kaanal-core/replay.md)           | Token bucket (GCRA) and slot gate with FIFO hand-over                                                                       |
| `src/balance.rs`          | [kaanal-core/balance.md](kaanal-core/balance.md)         | Replica count and balance policy (`round-robin`, `random`, `weighted`)                                          |
| `src/visits.rs`           | [kaanal-core/visits.md](kaanal-core/visits.md)           | Expected visits per node from the compiled chains (branch `prob`, `repeat`, cache hits)                         |
| `src/rng.rs`              | [kaanal-core/rng.md](kaanal-core/rng.md)                 | `HopRng`, the cheap per-hop RNG (SplitMix64)                                                                                |
| `src/dist.rs`             | [kaanal-core/dist.md](kaanal-core/dist.md)               | Fixed & lognormal latency distributions                                                                                     |
| `src/chaos/`              | [kaanal-core/chaos.md](kaanal-core/chaos.md)             | Chaos layers, `ChaosState`, compiled `EffectiveChaos` / `Resolved`                                                          |
| `src/scenario/`           | [kaanal-core/scenario.md](kaanal-core/scenario.md)       | Scenarios: names, `ScenarioSet`, `ChaosStack`, `resolve_scenario`                                                           |
| `src/chain/mod.rs`        | [kaanal-core/chain.md](kaanal-core/chain.md)             | Endpoint compile: default chains, via/with, limits, error_map lookup                                                        |
| `src/chain/types.rs`      | [kaanal-core/chain.md](kaanal-core/chain.md)             | `Step`, `Call`, `CacheStep`, `Seq`, `Endpoint`, `Endpoints`, `CollectionOp`, `RouteSpec`, `EndpointsInput`, `resolve_error` |
| `src/chain/place.rs`      | [kaanal-core/chain.md](kaanal-core/chain.md)             | `Place` (where a step sits, for positioned errors), `Keys`, `Job`, `Entry`                                                  |
| `src/chain/compiler.rs`   | [kaanal-core/chain.md](kaanal-core/chain.md)             | `compile`, `skeleton`, the limits check and the `Compiler` state                                                            |
| `src/chain/entries.rs`    | [kaanal-core/chain.md](kaanal-core/chain.md)             | `Compiler` methods that list the endpoints and their jobs, built-in and user                                                |
| `src/chain/steps.rs`      | [kaanal-core/chain.md](kaanal-core/chain.md)             | `Compiler` methods that compile one step: `call`, `cache`, `parallel`, `repeat`                                             |
| `src/chain/endpoint.rs`   | [kaanal-core/chain.md](kaanal-core/chain.md)             | `Compiler` methods that compile a whole endpoint: node chains, state overrides, heads                                       |
| `src/exec/`               | [kaanal-core/exec.md](kaanal-core/exec.md)               | The chain executor (`execute`), ops, hops, spans, determinism                                                               |
| `src/pin.rs`              | [kaanal-core/pin.md](kaanal-core/pin.md)                 | UI-state pins: `PinState`, layers, `states:` overrides, `resolve`                                                           |

## [kaanal-data](kaanal-data/) (OpenAPI + data generation)

| File                      | Page                                                   | Contents                                                                        |
| ------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------- |
| `src/lib.rs`              | [kaanal-data/lib.md](kaanal-data/lib.md)               | Crate root, re-exports                                                          |
| `src/api.rs`              | [kaanal-data/api.md](kaanal-data/api.md)               | OpenAPI 3.0 → `ApiModel`, `$ref` inlining with bomb guards                      |
| `src/generate/mod.rs`     | [kaanal-data/generate.md](kaanal-data/generate.md)     | Deterministic realistic JSON from a schema                                      |
| `src/generate/numbers.rs` | [kaanal-data/generate.md](kaanal-data/generate.md)     | Integers, numbers, arrays: bounds, counts, rounding                             |
| `src/generate/format.rs`  | [kaanal-data/generate.md](kaanal-data/generate.md)     | `format` strings and dictionary values                                          |
| `src/generate/text.rs`    | [kaanal-data/generate.md](kaanal-data/generate.md)     | Text values: names, addresses, phones, emails, ids, base64, dates, `fit_length` |
| `src/empty.rs`            | [kaanal-data/empty.md](kaanal-data/empty.md)           | The "no data" form of a body (`empty`, `empty_of_value`)                        |
| `src/warn.rs`             | [kaanal-data/warn.md](kaanal-data/warn.md)             | Ignored-keyword warnings                                                        |
| `src/words.rs`            | [kaanal-data/words.md](kaanal-data/words.md)           | The lorem word list (names and places are in packs)                             |
| `src/page/mod.rs`         | [kaanal-data/page.md](kaanal-data/page.md)             | Paging style detection (`Style`, `PageSpec::detect`)                            |
| `src/pack.rs`             | [kaanal-data/pack.md](kaanal-data/pack.md)             | Locale packs: names, places, phone, currency, `DataError`                       |
| `src/dictionary/`         | [kaanal-data/dictionary.md](kaanal-data/dictionary.md) | The field dictionary and its generators                                         |
| `src/dataset.rs`          | [kaanal-data/dataset.md](kaanal-data/dataset.md)       | `Options` and `DataSet`: loading `locales/` and `data.yaml`                     |
| `src/selfcheck.rs`        | [kaanal-data/selfcheck.md](kaanal-data/selfcheck.md)   | Self-validation of generated responses against their schema                     |

## [kaanal-server](kaanal-server/) (HTTP)

| File                      | Page                                                         | Contents                                                                                                                        |
| ------------------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`              | [kaanal-server/lib.md](kaanal-server/lib.md)                 | `serve`/`check`, router, port binding                                                                                           |
| `src/actor.rs`            | [kaanal-server/actor.md](kaanal-server/actor.md)             | Live replay-engine actor task and its handle                                                                                    |
| `src/auth.rs`             | [kaanal-server/auth.md](kaanal-server/auth.md)               | Signing key, JWKS, RS256 tokens, login                                                                                          |
| `src/control/mod.rs`      | [kaanal-server/control.md](kaanal-server/control.md)         | Control API handlers + the auth/Host guard                                                                                      |
| `src/control/guard.rs`    | [kaanal-server/control.md](kaanal-server/control.md)         | The security guard: Host check, bearer token, constant-time compare                                                             |
| `src/control/chaos.rs`    | [kaanal-server/control.md](kaanal-server/control.md)         | The chaos handlers (`get_chaos`, `set_chaos`, `set_chaos_keyed`, `clear_node`, `clear_keyed`, `topology`) and their patch types |
| `src/control/view.rs`     | [kaanal-server/control.md](kaanal-server/control.md)         | JSON views of chaos layers and the effective chaos, and the key and path helpers                                                |
| `src/control/scenario.rs` | [kaanal-server/control.md](kaanal-server/control.md)         | The scenario handlers: list, get, put, delete, activate (ttl lease), deactivate, the active stack                               |
| `src/control/lease.rs`    | [kaanal-server/control.md](kaanal-server/control.md)         | Lease expiry: `expire_due`, `expire_chaos`, `next_deadline` and the background `lease_task`                                     |
| `src/control/save.rs`     | [kaanal-server/control.md](kaanal-server/control.md)         | `POST /scenarios/save`: live, effective or a named scenario to a file                                                           |
| `src/control/admin.rs`    | [kaanal-server/control.md](kaanal-server/control.md)         | `reset`, `shutdown`, the `events` stream and the `ui` page                                                                      |
| `src/control/tests.rs`    | [kaanal-server/control.md](kaanal-server/control.md)         | The unit tests (see the last section)                                                                                           |
| `src/chaos_lease.rs`      | [kaanal-server/chaos_lease.md](kaanal-server/chaos_lease.md) | Leases on live chaos (`ttl`): units, table, pruning                                                                             |
| `src/mock/mod.rs`         | [kaanal-server/mock.md](kaanal-server/mock.md)               | Mock request handler, request ids, CORS                                                                                         |
| `src/mock/flight.rs`      | [kaanal-server/mock.md](kaanal-server/mock.md)               | One request in flight: log entry, identity, early exits                                                                         |
| `src/mock/reply.rs`       | [kaanal-server/mock.md](kaanal-server/mock.md)               | Building the answer: chaos, pins, wire failures, CORS                                                                           |
| `src/scenarios.rs`        | [kaanal-server/scenarios.md](kaanal-server/scenarios.md)     | Scenario file discovery and atomic save                                                                                         |
| `src/state.rs`            | [kaanal-server/state.md](kaanal-server/state.md)             | `AppState` and its shell state; `build` calls `Generation::build`                                                                                |
| `src/generation.rs`      | [kaanal-server/generation.md](kaanal-server/generation.md)   | `Generation::build` load pipeline (moved out of `state.rs`)                                                                       |
| `src/draft/mod.rs`        | [kaanal-server/draft.md](kaanal-server/draft.md)             | The Studio draft: text, `dirty`, `DraftSource`                                                                                  |
| `src/wire.rs`             | [kaanal-server/wire.md](kaanal-server/wire.md)               | `Cut` body stream for the `reset` failure kind                                                                                  |
| `src/scope.rs`            | [kaanal-server/scope.md](kaanal-server/scope.md)             | `X-Kaanal-Scenario` scoping and its stack cache                                                                                 |
| `src/pins.rs`             | [kaanal-server/pins.md](kaanal-server/pins.md)               | `empty_form`, `empty_ok`: what an `empty` pin answers                                                                           |
| `src/pin_api.rs`          | [kaanal-server/pin_api.md](kaanal-server/pin_api.md)         | `/pins` and `/cycle` routes                                                                                                     |
| `src/log.rs`              | [kaanal-server/log.md](kaanal-server/log.md)                 | Request ids, the 1000-entry log ring, query filters                                                                             |
| `src/record/mod.rs`       | [kaanal-server/record.md](kaanal-server/record.md)           | Trace text, log entry, flags and the bus event                                                                                  |
| `src/log_api.rs`          | [kaanal-server/log_api.md](kaanal-server/log_api.md)         | `/logs` and `/trace/{id}` routes                                                                                                |
| `src/shutdown.rs`         | [kaanal-server/shutdown.md](kaanal-server/shutdown.md)       | The graceful stop: deadline handle, cutting listener                                                                            |
| `src/ui.html`             | [../../web/web-ui.md](../../web/web-ui.md)                   | Embedded status page (documented with the web docs)                                                                             |

## [kaanal-sim](kaanal-sim/) (discrete-event simulator)

| File            | Page                                         | Contents                                     |
| --------------- | -------------------------------------------- | -------------------------------------------- |
| `src/lib.rs`    | [kaanal-sim/lib.md](kaanal-sim/lib.md)       | `run`, config and report types, validation   |
| `src/engine.rs` | [kaanal-sim/engine.md](kaanal-sim/engine.md) | The event loop: arrivals, FIFO slots, report |

## [kaanal](kaanal/) (binary)

| File                                 | Page                                                         | Contents                                                                                                                 |
| ------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `src/main.rs`                        | [kaanal/main.md](kaanal/main.md)                             | Entry point and dispatch (the commands are in the modules below)                                                         |
| `src/commands/chaos.rs`              | [kaanal/chaos_cmd.md](kaanal/chaos_cmd.md)                   | `kaanal chaos`, `node`, `cycle` and `pins`: patch bodies, paths and the text views of live chaos and pins                |
| `src/cli.rs`                         | [kaanal/cli.md](kaanal/cli.md)                               | The command-line shape: clap types for every subcommand, the `Cli` parse fix-ups, `schema` and `completions`             |
| `src/commands/down.rs`               | [kaanal/down.md](kaanal/down.md)                             | `kaanal down`: stop a local instance (drain, then force) or ask a remote one to shut down                                |
| `src/util/load.rs`                   | [kaanal/load.md](kaanal/load.md)                             | Loading and checking `kaanal.yaml`: capped reads, positioned config errors, the auth user list                           |
| `src/commands/logs.rs`               | [kaanal/logs_cmd.md](kaanal/logs_cmd.md)                     | `kaanal logs` and `trace`: one-shot and `--follow` reading of the request log, and the output helpers                    |
| `src/util/parse.rs`                  | [kaanal/parse.md](kaanal/parse.md)                           | Small argument parsers and text cleaners shared by the commands (durations, rates, path segments, control-text cleaning) |
| `src/instance/remote.rs`             | [kaanal/remote.md](kaanal/remote.md)                         | The thin HTTP client for the control API: one agent, one `call`, `status` and the shutdown reply                         |
| `src/commands/draft.rs`              | [kaanal/draft_cmd.md](kaanal/draft_cmd.md)                   | `kaanal draft`: show, set, validate and apply the instance's draft                                                       |
| `src/commands/scenario.rs`           | [kaanal/scenario_cmd.md](kaanal/scenario_cmd.md)             | `kaanal scenario` and `kaanal endpoints`: list, activate, save and print scenarios, and write missing endpoints          |
| `src/commands/serve.rs`              | [kaanal/serve.md](kaanal/serve.md)                           | `kaanal up`: start the server in the foreground or detached, the startup banner and the exposure warnings                |
| `src/instance/state_dir.rs`          | [kaanal/state_dir.md](kaanal/state_dir.md)                   | The `.kaanal/` state directory: a private folder, the one-instance lock and `run.json`                                   |
| `src/commands/init/mod.rs`           | [kaanal/init.md](kaanal/init.md)                             | `kaanal init`: prompts, config rendering, checked write                                                                  |
| `src/commands/init/fetch.rs`         | [kaanal/fetch.md](kaanal/fetch.md)                           | `kaanal init URL`: the limited spec download and its URL policy                                                          |
| `src/commands/demo/mod.rs`           | [kaanal/demo.md](kaanal/demo.md)                             | `kaanal demo`: checks, the template write, the banner, `--open`                                                          |
| `src/commands/demo/files.rs`         | [kaanal/demo.md](kaanal/demo.md)                             | The embedded demo project (`FILES`), from `crates/kaanal/demo/`                                                          |
| `src/instance/keys.rs`               | [kaanal/keys.md](kaanal/keys.md)                             | Signing key file: location, identity, race-safe creation, permissions                                                    |
| `src/util/logview/mod.rs`            | [kaanal/logview.md](kaanal/logview.md)                       | Log lines, the waterfall, the `logs -f` follow machinery                                                                 |
| `src/instance/lifecycle/`            | [kaanal/lifecycle.md](kaanal/lifecycle.md)                   | Liveness by lock, `run.json` reader, stopping, detaching, signals                                                        |
| `src/instance/lifecycle/liveness.rs` | [kaanal/lifecycle_liveness.md](kaanal/lifecycle_liveness.md) | Is an instance running: the lock probe, the `run.json` reader and the stale-state cleanup                                |
| `src/instance/lifecycle/pid.rs`      | [kaanal/lifecycle_pid.md](kaanal/lifecycle_pid.md)           | Telling whether a pid is a `kaanal` process (`tasklist` / `ps` parsing) and killing it                                   |
| `src/instance/lifecycle/stop.rs`     | [kaanal/lifecycle_stop.md](kaanal/lifecycle_stop.md)         | Stopping an instance: waiting for it, the forced stop, and the stop signals of the server process                        |
| `src/instance/lifecycle/spawn.rs`    | [kaanal/lifecycle_spawn.md](kaanal/lifecycle_spawn.md)       | Starting the background instance for `up -d`: the log rotation and the detached spawn                                    |
| `src/instance/lifecycle/ready.rs`    | [kaanal/lifecycle_ready.md](kaanal/lifecycle_ready.md)       | Waiting for a started instance to answer: polling, the unready settle, and the existing-instance check                   |
| `src/util/output.rs`                 | [kaanal/output.md](kaanal/output.md)                         | The global `--json` switch and `emit`                                                                                    |
| `src/instance/target.rs`             | [kaanal/target.md](kaanal/target.md)                         | `--endpoint` / `--token`: local or remote target, errors, warning                                                        |

## [kaanal-winhandle](kaanal-winhandle/) (the one `unsafe` crate)

| File         | Page                                               | Contents                                                          |
| ------------ | -------------------------------------------------- | ----------------------------------------------------------------- |
| `src/lib.rs` | [kaanal-winhandle/lib.md](kaanal-winhandle/lib.md) | `without_inheritable_stdio`: keeps `up -d` off the caller's pipes |
