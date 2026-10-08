# `crates/kaanal-core/src/config.rs`

Parsing of `kaanal.yaml` into typed config.

## What it does

Defines the serde types that mirror the YAML file, plus `Config::from_yaml` which parses and reports errors as `file:line:col: message` (rule R6.1 — the caller prepends the file name). Unknown fields are rejected everywhere (`deny_unknown_fields`) so typos fail loudly instead of being silently ignored.

## Types (YAML shape)

```yaml
version: 1            # Config.version — must be 1
seed: 42              # Config.seed — RNG seed, default 0
port: 8080            # Config.port — default 8080
nodes:                # IndexMap<String, NodeConfig> — insertion order preserved
  app:
    type: service     # NodeKind: gateway | service | database
    flavor: postgres  # cosmetic label
    proc: 5ms         # processing time: DistConfig
    spec: openapi.yaml
    ops:              # database only: read/write/scan costs
      read: { p50: 4ms, p99: 20ms }
edges:                # Vec<EdgeConfig>
  - { from: nginx, to: app, net: 1ms }
routes:               # IndexMap<String, RouteOverride> — static overrides
  "GET /health": { status: 200, body: { ok: true } }
```

## Items

| Item                     | Kind                 | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------ | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Config`                 | struct               | Top-level file. `version`, `seed` (default 0), `port` (default 8080 via `default_port`), `nodes`, `edges`, `routes`.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `NodeKind`               | enum                 | `Gateway` \| `Service` \| `Database` (lowercase in YAML).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `NodeConfig`             | struct               | One node: `kind` (YAML key `type`), `flavor`, `proc`, `spec`, `ops`, `concurrency` — all optional except `kind`.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `NodeConfig.concurrency` | `Option<NonZeroU32>` | Most simultaneous own hops (proc/op). `0`, negatives and fractions are rejected with a position. Honoured by the simulator; live mode warns and ignores it until M1.                                                                                                                                                                                                                                                                                                                                                                                                 |
| `OpsConfig`              | struct               | `read`/`write`/`scan` distributions, all required when `ops` is present.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `EdgeConfig`             | struct               | `from`, `to` node names, optional `net` latency.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `DistConfig`             | enum                 | A duration in YAML: `Fixed(Duration)` (`5ms`), `Pct { p50, p99 }` (`{ p50: 4ms, p99: 20ms }`) or `Exp { mean }` (`{ mean: 5ms }`). Custom `Deserialize` impl replaces serde's useless untagged-enum error with "expected a duration like `5ms`, `{ p50: 1ms, p99: 3ms }` or `{ mean: 5ms }`". Map shapes reject unknown keys, so `{ mean: 5ms, p99: 9ms }` is an error. The 1 h cap applies to `mean` too. Durations use `humantime_serde`. Any duration (or p99) over `MAX_DURATION` = 1 h is rejected with its line, so summed hops can never overflow `Duration`. |
| `RouteOverride`          | struct               | Static response for a `"METHOD /path"` key: `status` (default 200) and `body` (any JSON).                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `ConfigError`            | struct               | `line`/`col` (optional) + `msg`. `Display` prints `line:col: msg` when a position is known.                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `Config::from_yaml(src)` | fn                   | Strips a UTF-8 BOM (Windows editors add one; YAML rejects it — rule R6.4), parses with `serde_yaml_ng`, maps serde's location into `ConfigError`, rejects `version != 1`.                                                                                                                                                                                                                                                                                                                                                                                            |

## Notes

This file only _parses_. Semantic validation (one gateway, DAG, edge shapes, ops on databases) happens in [topology.md](topology.md); route/spec wiring happens in [state.md](../kaanal-server/state.md).
