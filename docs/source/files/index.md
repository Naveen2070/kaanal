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

## [kaanal-data](kaanal-data/) (OpenAPI + data generation)

| File              | Page                                               | Contents                                                   |
| ----------------- | -------------------------------------------------- | ---------------------------------------------------------- |
| `src/lib.rs`      | [kaanal-data/lib.md](kaanal-data/lib.md)           | Crate root, re-exports                                     |
| `src/api.rs`      | [kaanal-data/api.md](kaanal-data/api.md)           | OpenAPI 3.0 → `ApiModel`, `$ref` inlining with bomb guards |
| `src/generate.rs` | [kaanal-data/generate.md](kaanal-data/generate.md) | Deterministic realistic JSON from a schema                 |
| `src/warn.rs`     | [kaanal-data/warn.md](kaanal-data/warn.md)         | Ignored-keyword warnings                                   |
| `src/words.rs`    | [kaanal-data/words.md](kaanal-data/words.md)       | Name/city/word lists (en-IN)                               |

## [kaanal-server](kaanal-server/) (HTTP)

| File               | Page                                                     | Contents                                            |
| ------------------ | -------------------------------------------------------- | --------------------------------------------------- |
| `src/lib.rs`       | [kaanal-server/lib.md](kaanal-server/lib.md)             | `serve`/`check`, router, port binding               |
| `src/actor.rs`     | [kaanal-server/actor.md](kaanal-server/actor.md)         | Live replay-engine actor task and its handle        |
| `src/auth.rs`      | [kaanal-server/auth.md](kaanal-server/auth.md)           | Signing key, JWKS, RS256 tokens, login              |
| `src/control.rs`   | [kaanal-server/control.md](kaanal-server/control.md)     | Control API handlers + the auth/Host guard          |
| `src/mock.rs`      | [kaanal-server/mock.md](kaanal-server/mock.md)           | Mock request handler, trace header, CORS            |
| `src/scenarios.rs` | [kaanal-server/scenarios.md](kaanal-server/scenarios.md) | Scenario file discovery and atomic save             |
| `src/state.rs`     | [kaanal-server/state.md](kaanal-server/state.md)         | `AppState::build` load pipeline, route conflicts    |
| `src/wire.rs`      | [kaanal-server/wire.md](kaanal-server/wire.md)           | `Cut` body stream for the `reset` failure kind      |
| `src/scope.rs`     | [kaanal-server/scope.md](kaanal-server/scope.md)         | `X-Kaanal-Scenario` scoping and its stack cache     |
| `src/ui.html`      | [../../web/web-ui.md](../../web/web-ui.md)               | Embedded status page (documented with the web docs) |

## [kaanal-sim](kaanal-sim/) (discrete-event simulator)

| File            | Page                                         | Contents                                     |
| --------------- | -------------------------------------------- | -------------------------------------------- |
| `src/lib.rs`    | [kaanal-sim/lib.md](kaanal-sim/lib.md)       | `run`, config and report types, validation   |
| `src/engine.rs` | [kaanal-sim/engine.md](kaanal-sim/engine.md) | The event loop: arrivals, FIFO slots, report |

## [kaanal](kaanal/) (binary)

| File          | Page                             | Contents                                                              |
| ------------- | -------------------------------- | --------------------------------------------------------------------- |
| `src/main.rs` | [kaanal/main.md](kaanal/main.md) | CLI, instance lock, run file, control-API client                      |
| `src/keys.rs` | [kaanal/keys.md](kaanal/keys.md) | Signing key file: location, identity, race-safe creation, permissions |
