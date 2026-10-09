# `crates/kaanal-core/src/topology.rs`

The topology graph: nodes, edges, and all semantic validation of the config.

## What it does

`Topology::build` turns a parsed `Config` into a validated graph the executor can walk. Anything the executor would silently ignore or misinterpret is **rejected at load** with a named error (rule R2.7, R6.1).

## Types

| Item            | Kind       | What it does                                                                                                                                                                                                                   |
| --------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `NodeId`        | type alias | `usize` — index into `Topology::nodes`.                                                                                                                                                                                        |
| `Node`          | struct     | Runtime node: `name`, `kind`, `flavor`, `spec`, `proc` distribution, `ops: Option<[Dist; 3]>` (read/write/scan, indexed by `Op as usize`), `concurrency: Option<NonZeroU32>` (most simultaneous own hops; `None` = unlimited). |
| `Edge`          | struct     | `from`, `to`, `net` latency distribution.                                                                                                                                                                                      |
| `Topology`      | struct     | `nodes`, `edges`, `gateway` (the one gateway's id), private `index` (name → id).                                                                                                                                               |
| `TopologyError` | enum       | Every rejection reason, each with human `Display`: `UnknownNode { edge, name }`, `GatewayCount(n)`, `Cycle(path)`, `MissingOps(name)`, `MissingGatewayEdge(name)`, `Unsupported(msg)`.                                         |

## `Topology::build` checks, in order

Field placement (`ops` on a service, `spec` on a gateway) is no longer checked here: each node kind has its own strict struct, so it is a parse error in [config.md](config.md).

1. **Database ops present, presets fill gaps** (monolith too when it sets `ops`/`flavor`; private `ops_of` via [presets.md](presets.md)) → `MissingOps`.
2. **Exactly one gateway** → else `GatewayCount(0 or n)`.
3. **Edge endpoints exist** → else `UnknownNode` (reports which edge number).
4. **DAG** — `check_cycles` runs an iterative-entry DFS with 3-colour marking (0 unvisited / 1 on path / 2 done); revisiting a colour-1 node reports the cycle as a readable path (`cycle: a -> b -> a`).
5. **Phase-one edge shapes**: every edge must be `gateway → service` or `service → database`; anything else → `Unsupported`. Duplicate edges are also rejected.
6. **Reachability**: a service with an OpenAPI `spec` but no edge from the gateway → `MissingGatewayEdge` (its routes would never be walked).

## Lookup methods

| Method           | What it does                        |
| ---------------- | ----------------------------------- |
| `id(name)`       | Name → `NodeId`.                    |
| `out_edges(id)`  | Iterator over edges leaving a node. |
| `edge(from, to)` | The specific edge, if any.          |
