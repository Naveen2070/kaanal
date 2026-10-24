# Per-file documentation

One page per source file: what the file does and what each function/type in it does. Grouped by crate.

## [kaanal-core](kaanal-core/) (pure logic)

| File                 | Page                                                     | Contents                                                              |
| -------------------- | -------------------------------------------------------- | --------------------------------------------------------------------- |
| `src/lib.rs`         | [kaanal-core/lib.md](kaanal-core/lib.md)                 | Crate root, re-exports, `stable_hash`                                 |
| `src/config.rs`      | [kaanal-core/config.md](kaanal-core/config.md)           | `kaanal.yaml` parsing: tagged node kinds, `DistConfig`, `ConfigError` |
| `src/diagnostics.rs` | [kaanal-core/diagnostics.md](kaanal-core/diagnostics.md) | Error positions: saphyr tree, per-branch schema check, `Located`      |
| `src/presets.rs`     | [kaanal-core/presets.md](kaanal-core/presets.md)         | Flavor presets (estimates) and gap filling                            |
| `src/schema.rs`      | [kaanal-core/schema.md](kaanal-core/schema.md)           | JSON Schema of `kaanal.yaml` (schemars)                               |
| `src/topology.rs`    | [kaanal-core/topology.md](kaanal-core/topology.md)       | Graph build + validation (optional gateway, edge table, DAG, presets) |
| `src/replay/`        | [kaanal-core/replay.md](kaanal-core/replay.md)           | Token bucket (GCRA) and slot gate with FIFO hand-over                 |
| `src/dist.rs`        | [kaanal-core/dist.md](kaanal-core/dist.md)               | Fixed & lognormal latency distributions                               |
| `src/chaos.rs`       | [kaanal-core/chaos.md](kaanal-core/chaos.md)             | Chaos layers, `ChaosState`, compiled `EffectiveChaos` / `Resolved`    |
| `src/scenario.rs`    | [kaanal-core/scenario.md](kaanal-core/scenario.md)       | Scenarios: names, `ScenarioSet`, `ChaosStack`, `resolve_scenario`     |
| `src/chain.rs`       | [kaanal-core/chain.md](kaanal-core/chain.md)             | Endpoint compile: default chains, via/with, limits, error_map lookup  |
| `src/exec.rs`        | [kaanal-core/exec.md](kaanal-core/exec.md)               | The chain executor (`execute`), ops, hops, spans, determinism         |
| `src/pin.rs`         | [kaanal-core/pin.md](kaanal-core/pin.md)                 | UI-state pins: `PinState`, layers, `states:` overrides, `resolve`     |

## [kaanal-data](kaanal-data/) (OpenAPI + data generation)

| File                | Page                                                   | Contents                                                    |
| ------------------- | ------------------------------------------------------ | ----------------------------------------------------------- |
| `src/lib.rs`        | [kaanal-data/lib.md](kaanal-data/lib.md)               | Crate root, re-exports                                      |
| `src/api.rs`        | [kaanal-data/api.md](kaanal-data/api.md)               | OpenAPI 3.0 → `ApiModel`, `$ref` inlining with bomb guards  |
| `src/generate.rs`   | [kaanal-data/generate.md](kaanal-data/generate.md)     | Deterministic realistic JSON from a schema                  |
| `src/empty.rs`      | [kaanal-data/empty.md](kaanal-data/empty.md)           | The "no data" form of a body (`empty`, `empty_of_value`)    |
| `src/warn.rs`       | [kaanal-data/warn.md](kaanal-data/warn.md)             | Ignored-keyword warnings                                    |
| `src/words.rs`      | [kaanal-data/words.md](kaanal-data/words.md)           | The lorem word list (names and places are in packs)         |
| `src/pack.rs`       | [kaanal-data/pack.md](kaanal-data/pack.md)             | Locale packs: names, places, phone, currency, `DataError`   |
| `src/dictionary.rs` | [kaanal-data/dictionary.md](kaanal-data/dictionary.md) | The field dictionary and its generators                     |
| `src/dataset.rs`    | [kaanal-data/dataset.md](kaanal-data/dataset.md)       | `Options` and `DataSet`: loading `locales/` and `data.yaml` |

## [kaanal-server](kaanal-server/) (HTTP)

| File                 | Page                                                         | Contents                                              |
| -------------------- | ------------------------------------------------------------ | ----------------------------------------------------- |
| `src/lib.rs`         | [kaanal-server/lib.md](kaanal-server/lib.md)                 | `serve`/`check`, router, port binding                 |
| `src/actor.rs`       | [kaanal-server/actor.md](kaanal-server/actor.md)             | Live replay-engine actor task and its handle          |
| `src/auth.rs`        | [kaanal-server/auth.md](kaanal-server/auth.md)               | Signing key, JWKS, RS256 tokens, login                |
| `src/control.rs`     | [kaanal-server/control.md](kaanal-server/control.md)         | Control API handlers + the auth/Host guard            |
| `src/chaos_lease.rs` | [kaanal-server/chaos_lease.md](kaanal-server/chaos_lease.md) | Leases on live chaos (`ttl`): units, table, pruning   |
| `src/mock.rs`        | [kaanal-server/mock.md](kaanal-server/mock.md)               | Mock request handler, request ids, CORS               |
| `src/scenarios.rs`   | [kaanal-server/scenarios.md](kaanal-server/scenarios.md)     | Scenario file discovery and atomic save               |
| `src/state.rs`       | [kaanal-server/state.md](kaanal-server/state.md)             | `AppState::build` load pipeline, route conflicts      |
| `src/wire.rs`        | [kaanal-server/wire.md](kaanal-server/wire.md)               | `Cut` body stream for the `reset` failure kind        |
| `src/scope.rs`       | [kaanal-server/scope.md](kaanal-server/scope.md)             | `X-Kaanal-Scenario` scoping and its stack cache       |
| `src/pins.rs`        | [kaanal-server/pins.md](kaanal-server/pins.md)               | `empty_form`, `empty_ok`: what an `empty` pin answers |
| `src/pin_api.rs`     | [kaanal-server/pin_api.md](kaanal-server/pin_api.md)         | `/pins` and `/cycle` routes                           |
| `src/log.rs`         | [kaanal-server/log.md](kaanal-server/log.md)                 | Request ids, the 1000-entry log ring, query filters   |
| `src/record.rs`      | [kaanal-server/record.md](kaanal-server/record.md)           | Trace text, log entry, flags and the bus event        |
| `src/log_api.rs`     | [kaanal-server/log_api.md](kaanal-server/log_api.md)         | `/logs` and `/trace/{id}` routes                      |
| `src/shutdown.rs`    | [kaanal-server/shutdown.md](kaanal-server/shutdown.md)       | The graceful stop: deadline handle, cutting listener  |
| `src/ui.html`        | [../../web/web-ui.md](../../web/web-ui.md)                   | Embedded status page (documented with the web docs)   |

## [kaanal-sim](kaanal-sim/) (discrete-event simulator)

| File            | Page                                         | Contents                                     |
| --------------- | -------------------------------------------- | -------------------------------------------- |
| `src/lib.rs`    | [kaanal-sim/lib.md](kaanal-sim/lib.md)       | `run`, config and report types, validation   |
| `src/engine.rs` | [kaanal-sim/engine.md](kaanal-sim/engine.md) | The event loop: arrivals, FIFO slots, report |

## [kaanal](kaanal/) (binary)

| File                | Page                                       | Contents                                                              |
| ------------------- | ------------------------------------------ | --------------------------------------------------------------------- |
| `src/main.rs`       | [kaanal/main.md](kaanal/main.md)           | CLI, instance lock, run file, control-API client                      |
| `src/init.rs`       | [kaanal/init.md](kaanal/init.md)           | `kaanal init`: prompts, config rendering, checked write               |
| `src/fetch.rs`      | [kaanal/fetch.md](kaanal/fetch.md)         | `kaanal init URL`: the limited spec download and its URL policy       |
| `src/demo.rs`       | [kaanal/demo.md](kaanal/demo.md)           | `kaanal demo`: checks, the template write, the banner, `--open`       |
| `src/demo_files.rs` | [kaanal/demo.md](kaanal/demo.md)           | The embedded demo project (`FILES`), from `crates/kaanal/demo/`       |
| `src/keys.rs`       | [kaanal/keys.md](kaanal/keys.md)           | Signing key file: location, identity, race-safe creation, permissions |
| `src/logview.rs`    | [kaanal/logview.md](kaanal/logview.md)     | Log lines, the waterfall, the `logs -f` follow machinery              |
| `src/lifecycle.rs`  | [kaanal/lifecycle.md](kaanal/lifecycle.md) | Liveness by lock, `run.json` reader, stopping, detaching, signals     |
| `src/output.rs`     | [kaanal/output.md](kaanal/output.md)       | The global `--json` switch and `emit`                                 |
| `src/target.rs`     | [kaanal/target.md](kaanal/target.md)       | `--endpoint` / `--token`: local or remote target, errors, warning     |

## [kaanal-winhandle](kaanal-winhandle/) (the one `unsafe` crate)

| File         | Page                                               | Contents                                                          |
| ------------ | -------------------------------------------------- | ----------------------------------------------------------------- |
| `src/lib.rs` | [kaanal-winhandle/lib.md](kaanal-winhandle/lib.md) | `without_inheritable_stdio`: keeps `up -d` off the caller's pipes |
