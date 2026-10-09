# `crates/kaanal-core/src/chaos.rs`

Chaos state types: the live, user-controlled failure knobs per node.

## What it does

Holds the _current_ chaos settings, keyed by node. Written by the control API (`PUT/DELETE /__kaanal/chaos...`), read by the chain executor on every request. Everything here is runtime state — none of it comes from `kaanal.yaml`.

## Items

| Item                    | Kind   | What it does                                                                                                                                                                                                                                                                                                    |
| ----------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NodeChaos`             | struct | Chaos settings for one node: `latency_add` (extra duration added to every visit of the node), `error_rate` (0.0–1.0 probability of failing), `error_status` (HTTP status to fail with), `down` (hard outage — every visit fails with 503 regardless of rate).                                                   |
| `MAX_LATENCY_ADD`       | const  | 5 min (300 000 ms): the longest `latency_add` chaos may set. A typo like 1e12 ms would otherwise hang every live request or overflow the simulator's clock.                                                                                                                                                     |
| `NodeChaos::validate()` | fn     | The one set of chaos limits (rule R6.9): `latency_add` ≤ `MAX_LATENCY_ADD`, `error_rate` within 0–1, `error_status` within 500–504 (4xx belongs to pins and cycle states, not random chaos). Returns a message naming the first bad field. Called by the server's `set_chaos` and by `kaanal-sim`'s `validate`. |
| `NodeChaos::default()`  | impl   | All-off defaults: zero latency, 0% errors, `error_status: 503`, `down: false`. An all-default entry is _inert_ — it changes nothing, which `exec` tests rely on.                                                                                                                                                |
| `ChaosState`            | type   | `HashMap<NodeId, NodeChaos>`. Absent node = no chaos.                                                                                                                                                                                                                                                           |

## Invariants

- Every driver runs `NodeChaos::validate` before anything lands here (the control API on the merged patch, the simulator on its whole `ChaosState`); the executor trusts the values.
- The executor reads this through a `std::sync::RwLock` held in `AppState` (see [server-state.md](../kaanal-server/state.md)).
