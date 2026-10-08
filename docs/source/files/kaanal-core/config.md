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

| Item | Kind | What it does |
|---|---|---|
| `Config` | struct | Top-level file. `version`, `seed` (default 0), `port` (default 8080 via `default_port`), `nodes`, `edges`, `routes`. |
| `NodeKind` | enum | `Gateway` \| `Service` \| `Database` (lowercase in YAML). |
| `NodeConfig` | struct | One node: `kind` (YAML key `type`), `flavor`, `proc`, `spec`, `ops` — all optional except `kind`. |
| `OpsConfig` | struct | `read`/`write`/`scan` distributions, all required when `ops` is present. |
| `EdgeConfig` | struct | `from`, `to` node names, optional `net` latency. |
| `DistConfig` | enum | A duration in YAML: either `Fixed(Duration)` (`5ms`) or `Pct { p50, p99 }` (`{ p50: 4ms, p99: 20ms }`). Custom `Deserialize` impl replaces serde's useless untagged-enum error with "expected a duration like `5ms`, or `{ p50: 1ms, p99: 3ms }`". Durations use `humantime_serde`. |
| `RouteOverride` | struct | Static response for a `"METHOD /path"` key: `status` (default 200) and `body` (any JSON). |
| `ConfigError` | struct | `line`/`col` (optional) + `msg`. `Display` prints `line:col: msg` when a position is known. |
| `Config::from_yaml(src)` | fn | Strips a UTF-8 BOM (Windows editors add one; YAML rejects it — rule R6.4), parses with `serde_yaml_ng`, maps serde's location into `ConfigError`, rejects `version != 1`. |

## Notes

This file only *parses*. Semantic validation (one gateway, DAG, edge shapes, ops on databases) happens in [core-topology.md](topology.md); route/spec wiring happens in [server-state.md](../kaanal-server/state.md).
