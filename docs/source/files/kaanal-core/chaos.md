# `crates/kaanal-core/src/chaos.rs`

Chaos state types: the live, user-controlled failure knobs per node.

## What it does

Holds the _current_ chaos settings, keyed by node. Written by the control API (`PUT/DELETE /__kaanal/chaos...`), read by the chain executor on every request. Everything here is runtime state — none of it comes from `kaanal.yaml`.

## Items

| Item                   | Kind   | What it does                                                                                                                                                                                                                                                  |
| ---------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NodeChaos`            | struct | Chaos settings for one node: `latency_add` (extra duration added to every visit of the node), `error_rate` (0.0–1.0 probability of failing), `error_status` (HTTP status to fail with), `down` (hard outage — every visit fails with 503 regardless of rate). |
| `NodeChaos::default()` | impl   | All-off defaults: zero latency, 0% errors, `error_status: 503`, `down: false`. An all-default entry is _inert_ — it changes nothing, which `exec` tests rely on.                                                                                              |
| `ChaosState`           | type   | `HashMap<NodeId, NodeChaos>`. Absent node = no chaos.                                                                                                                                                                                                         |

## Invariants

- `error_rate` is validated to 0.0–1.0 and `error_status` to 500–504 by the control API before anything lands here; the executor trusts the values.
- The executor reads this through a `std::sync::RwLock` held in `AppState` (see [server-state.md](../kaanal-server/state.md)).
