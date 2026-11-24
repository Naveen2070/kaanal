# `crates/kaanal-core/src/resize/`

The sizing planner's candidates (Step 9c): the size moves each node offers, the database-family alternatives, and how one is applied to a cloned `Config`. Nothing here runs a simulation or reads a file; the check and the walk live in `kaanal-sim`.

## Files

| File          | Contents                                                                                                         |
| ------------- | ---------------------------------------------------------------------------------------------------------------- |
| `mod.rs`      | Module root; re-exports the public surface of `moves.rs`, `family.rs` and `provider.rs`                          |
| `moves.rs`    | `Move`, `MoveKind`, the move rules (`moves_for`, `moves_for_oom`, `why_fixed`, `copies_only_reason`) and `apply` |
| `family.rs`   | `Family`, `family_of`, `Alternative`, `Alternatives` and `alternatives`: a database as one of the other family   |
| `provider.rs` | `Translation`, `providers_of` and `translate`: the design moved to another cloud's classes (Step 9c-2)           |
| `cheaper.rs`  | `cheaper_moves`: every single-knob move to a strictly cheaper value, cheapest first (Step 9c-2)                  |

## The move rules

One step of each knob, per node:

| Node                                                          | Moves                                                                                                                                                                            |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| In `keep` (pinned by the developer)                           | none (`"kept by you"`)                                                                                                                                                           |
| External API, object store                                    | none (`"an external service"`, `"an object store"`): no machine, no copies to buy                                                                                                |
| A table instance, nothing hand-written                        | `Class` (the next rung of `instances::ladder_by`: same series, priced from the list the node's hosting and engine use, more vCPU and no less memory) and `Copies`; never `Cores` |
| A table instance with its own `cpu` or its own `memory.limit` | `Copies` only: a larger class would leave the hand-written figure in force                                                                                                       |
| A custom instance (a name from `instances:`)                  | `Copies` only: its figures are written into the node at load, so own and filled cannot be told apart                                                                             |
| No instance, `cpu` written                                    | `Cores` (the next of 1, 2, 4, 8, 16, 32, 64 above the current) and `Copies`                                                                                                      |
| No instance, no `cpu`                                         | `Copies` only: its cores are not limited, so a core count would only cap it                                                                                                      |
| A cache                                                       | `Copies` only: its cores are not modelled (RULES 122)                                                                                                                            |

`Copies` adds one, stops at `MAX_REPLICAS` (64) and is refused where a `weighted` balance on the node, or on an edge into it, names each copy, and on a node with a `rate_limit` (each copy brings its own limiter, so a copy would raise the limit you set). `moves_for_oom` is the same with memory in place of cores: the next class up on a node with a table instance (nothing hand-written), else the written `memory.limit` doubled.

`why_fixed` gives the plain reason a node has no move at all (`None` when it has one); with both knobs blocked it gives the copies reason, the one a developer can unblock. `copies_only_reason` gives why a node with a machine moves by copies only, so a step can say so.

`apply` checks the move's `from` is what the node has (a stale move is refused), writes the field (`instance`, `cpu.cores`, `memory.limit`, `replicas`), then calls `Config::validate_candidate` (the memory limit against the class and the cores' range from `check_edges`, the replica range and a weighted balance from `check_replicas`). On any failure the config is left as it was.

## Database-family alternatives

`alternatives(cfg, node, keep, with_graph)` clones the design with database `node` turned into each flavor of the other family (relational: postgres, mysql, mariadb, sqlserver, oracle; non-relational: mongodb, dynamodb, cassandra, and neo4j only when `with_graph`). A flavor changes the simulation only through its preset ops, so a node with `ops` written gets `Err("your own ops are kept: ...")`; a node in `keep`, a non-database or an unknown node gets its own reason.

Hosting follows the new flavor. A managed node stays managed where the flavor has a managed engine (a written `engine:` that does not run it goes back to the default). Where it has none (cassandra, dynamodb) the node becomes `self`, `engine` and `multi_az` are cleared, and the class turns into its compute twin (`aws/db.m5.large` to `aws/m5.large`; no instance when the table has no twin), each with a note. A candidate that fails `Config::validate_candidate` (which now includes the hosting rules), or keeps a table class with no price for the new engine (an Azure class is Azure SQL only), goes to `Alternatives::dropped` with the reason; the rest are in `kept`.

## Provider translation

`translate(cfg, to, keep)` clones the design with each table class moved to the **cheapest priced class of `to`** whose vCPU and memory are each at least the original's, and whose network rate is at least the original's when both give one (`net_mbps` 0 is not compared). A price tie goes to the smaller class (fewer vCPU, then less memory), then the first name. A managed database goes to a managed class of `to` only where its flavor runs there (Azure: Azure SQL, so `sqlserver` only; GCP: no managed class in the table), and a written `engine` is rewritten to one the new cloud runs; otherwise the whole translation fails with `"<node>: no equivalent on <to>"`. A node in `keep` or a node translation cannot move (a cache, an object store, an external API, a serverless, external or managed non-database node) that carries a table class makes the whole translation fail `"<node>: kept on <provider>, so a section on <to> would mix providers"` (or `"<node>: <why> on <provider> has no translation, ..."`), so a section is never a mixed design; the same nodes with no class, a node with no class and a custom instance are kept as written, each with a note `"<node>: kept as written, not compared (why)"`. The region of `to` follows `cost::regions_of`: the file's `region:` for `to`, else the cloud's default (`prices::DEFAULT_REGIONS`); `region:` is not changed. Translating to a provider the design already uses is `Err("already on <to>")`; the result passes `Config::validate_candidate`. `providers_of(cfg)` lists the providers of the design's table classes, sorted and deduplicated.

## Cheaper moves

`cheaper_moves(cfg, node, keep)` is the shrinking side of the planner, for the cheapest search. For a table class it gives the **Pareto front** over (vCPU up, memory up, price down) of the classes of the node's provider and sort (compute, managed database or managed cache, each from its own price list) that cost less than the current one, of any series (Kung, Luccio and Preparata 1975; the skyline operator, Borzsonyi et al. 2001); twins with the same vCPU, memory and price keep the first name. It also gives copies from 1 up to one below the current count (where copies may change at all: no `rate_limit`, no `weighted` balance). The list is ordered ascending by the price of the resulting node (class price times copies). It is empty for a node in `keep`, a node with no instance (it has no price, and the cheapest search never shrinks what costs nothing, RULES 142 e), a custom instance, a class with no price, an external API or an object store. As in `moves_for`, a class is not offered where `cpu` or `memory.limit` is written. `moves_for` and every 9c-1 answer are unchanged.

## Public surface

| Item                  | Kind     | Notes                                                                                                                          |
| --------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `MoveKind`            | enum     | `Class { from, to }`, `Cores { from, to }`, `Memory { from, to }` (bytes), `Copies { from, to }`; serialized with a `knob` tag |
| `Move`                | struct   | `node`, `kind`                                                                                                                 |
| `moves_for`           | fn       | `(&Config, node, keep) -> Vec<Move>`                                                                                           |
| `moves_for_oom`       | fn       | same, memory in place of cores                                                                                                 |
| `why_fixed`           | fn       | `(&Config, node, keep) -> Option<&'static str>`                                                                                |
| `copies_only_reason`  | fn       | `(&Config, node, keep) -> Option<&'static str>`                                                                                |
| `apply`               | fn       | `(&mut Config, &Move) -> Result<(), String>`                                                                                   |
| `alternatives`        | fn       | `(&Config, node, keep, with_graph) -> Result<Alternatives, &'static str>`                                                      |
| `Alternatives`        | struct   | `kept: Vec<Alternative>`, `dropped: Vec<(DbFlavor, String)>`                                                                   |
| `Alternative`         | struct   | `node`, `flavor`, `config` (the whole candidate), `notes`                                                                      |
| `Family`, `family_of` | enum, fn | relational or non-relational, per `DbFlavor`                                                                                   |
| `cheaper_moves`       | fn       | `(&Config, node, keep) -> Vec<Move>`, cheaper values only, cheapest first                                                      |
| `translate`           | fn       | `(&Config, to, keep) -> Result<Translation, String>`                                                                           |
| `Translation`         | struct   | `config` (the whole candidate), `notes`                                                                                        |
| `providers_of`        | fn       | `(&Config) -> Vec<String>`                                                                                                     |

Tests: `crates/kaanal-core/tests/resize.rs`, `crates/kaanal-core/tests/resize_family.rs`, `crates/kaanal-core/tests/resize_provider.rs`.
