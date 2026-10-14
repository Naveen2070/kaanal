# `crates/kaanal-core/src/chain.rs`

Compiles `endpoints:`, node `chains:` and the topology into resolved call trees, once at load. The executor walks the result with no name lookups.

## What it does

`compile(topo, routes, input)` takes the routes the specs define (the server reads the spec files), the `endpoints:` entries and the node `chains:`, and returns `Endpoints`: one `Endpoint` per route. Every node name, edge, `via:`, `with:` and limit is resolved and checked here, so a bad chain fails at load with a place, never mid-request.

## Types

| Item                                     | Kind   | What it does                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Step`                                   | enum   | `Proc`, `Op`, `Call`, `Cache`, `Parallel`: one resolved step.                                                                                                                                                                                                                                                                                                                     |
| `Call`                                   | struct | Callee, `repeat`, edge `net`, shared `body`, optional fixed `op`, `key`.                                                                                                                                                                                                                                                                                                          |
| `CacheStep`                              | struct | Cache node, edge `net`, `reads_only` (default-chain lookups skip writes), `repeat` (lookups in sequence: a stored count, so a large edge `calls` stays cheap), `key`.                                                                                                                                                                                                             |
| `Seq`                                    | struct | Steps plus `hops` and `depth`, computed bottom-up (saturating) by `Seq::new`.                                                                                                                                                                                                                                                                                                     |
| `StepKey`                                | struct | `(target, occ)`: the n-th call of a target in an endpoint; the seed of its draws.                                                                                                                                                                                                                                                                                                 |
| `Endpoint`                               | struct | `key`, `owner`, `entry_node` (the gateway if the topology has one, else the owner; where chaos at the front door lands), `op`, `chain` (gateway work + owner body), `body`, `error_map`, `response`, `synthesized`, `responder`; `located(msg)` places an error; `is_wire_node(topo, node)` is true for the gateway and the owner (B2a Z7, see [../../chaos.md](../../chaos.md)). |
| `Responder`                              | enum   | `Login` or `Jwks`: which built-in body the server builds once the route's chain succeeds. Plain data; core never builds tokens.                                                                                                                                                                                                                                                   |
| `LOGIN_KEY`, `JWKS_KEY`                  | const  | `"POST /login"` and `"GET /.well-known/jwks.json"`, the keys of an auth node's built-in routes.                                                                                                                                                                                                                                                                                   |
| `Endpoints`                              | struct | Endpoints sorted by key (`get`, `iter`) and `warnings`.                                                                                                                                                                                                                                                                                                                           |
| `RouteSpec`                              | struct | A spec route: `key`, `owner`, `op`; `RouteSpec::inferred(key, owner)` infers the op.                                                                                                                                                                                                                                                                                              |
| `EndpointsInput`                         | struct | The entries and the file they came from; `EndpointsInput::inline(cfg)` for an inline map.                                                                                                                                                                                                                                                                                         |
| `resolve_error(map, node, kind, status)` | fn     | `error_map` lookup: failing node, then failure kind (`reset`, `malformed`, `timeout`), then exact status, then class (`5xx`); the status/class tiers see the kind's default status. No match keeps that default and no body.                                                                                                                                                      |
| `default_status(kind, status)`           | fn     | The status a failure answers with when nothing matches: the failure's own for `Status`, 502 for `Reset` and `Malformed`, 504 for `Timeout` (B2a Z7).                                                                                                                                                                                                                              |

## Default chain

Built once per node, callees first, shared through `Arc`, and op-free: a database step runs the op of its caller, so one tree serves reads and writes.

- A node's own work always comes first: its `proc` (plus its inline op for a monolith with inline ops).
- Service and monolith: callees in edge order (service, monolith, external API, auth), then cache lookups (an edge with `calls: N` gives N lookups, marked `reads_only`, so writes skip them), then databases.
- Database: its op. External API and auth: their `proc` only. Gateway: `proc` plus one call per auth edge. A cache is never a body; it is reached by `cache` steps.
- A custom `chain:` replaces the walk but keeps the owner's own work first.

## Compile order

Default bodies callees-first; then per node (callees-first) its named chains and the endpoints it owns; then static endpoints; then `HEAD` copies. A `via:` target is therefore always compiled before its caller, so `via:` cannot cycle.

## Step keys

Each call or cache step counts occurrences per target inside one sequence (a body, an inline or named chain, or the gateway part of a chain), not across the whole endpoint; `parallel` is transparent, so branch steps count as siblings in branch order. `chain: [ db, { cache: cache }, db, { parallel: [ db, orders ] } ]` gives `db#0 cache#0 db#1 db#2 orders#0`. A body shared by several callers (a diamond) repeats its own counts, so seeding must chain the parent step's path (D8) to tell the calls apart.

## Limits

| Limit                        | Value   | Checked                                                   |
| ---------------------------- | ------- | --------------------------------------------------------- |
| Steps per chain              | 256     | each list                                                 |
| Branches per `parallel`      | 32      | each `parallel`                                           |
| Nesting (`parallel`, inline) | 16      | each step                                                 |
| `repeat` and edge `calls`    | 1 000   | each step; each edge (edge `calls` is a default `repeat`) |
| Resolved hops                | 100 000 | each endpoint and named chain, after expanding everything |
| Resolved call depth          | 64      | same                                                      |

## Errors

Messages start with `endpoint 'KEY':` (or `node 'N': chain 'C':`) and point at the step, key or value that is wrong, in `kaanal.yaml` or in the endpoints file. Covered: malformed key, unknown method, duplicate route, bad statuses and `error_map` keys, unknown node, missing edge, wrong target kind, misplaced `op`, `repeat` or `with`/`via`/`chain`, unknown `with`/`via` target, empty chain, static route without a gateway, and every limit.

Warnings (not errors): a stale entry (no spec route, no `override`), a `HEAD` entry of a static `GET` entry (the copy of that `GET` is served), and a `via:` target whose `error_map` or `override` is ignored.

A stale entry is skipped, so a broken `chain:` in it only warns, but its `error_map` keys and statuses are still checked at load. Routes passed to `compile` must have unique keys (the server rejects duplicates first; a duplicate trips a debug assertion).
Two limits to know: the default body of every node is built, used or not, so an edge with `calls: 1000` to a cache stores 1 000 cache steps (A2 Task 3 revisits this), and `compile` does not check that two routes differ only in parameter names (`/users/{id}` vs `/users/{uid}`) or use the reserved `/__kaanal` prefix; the server keeps that check (A2 Task 4).

## Built-in routes

With an auth node, `compile` always adds `POST /login` and `GET /.well-known/jwks.json` (and the synthesized `HEAD` of the JWKS), owned by the auth node, op `read`, each with its `Responder`. They are listed after the spec routes and before stale detection, so an `endpoints:` entry for them (`error_map`, `override`, `chain`) customises them instead of being stale.

| Topology                              | Chain                                                                                                                                          |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Gateway with a `gateway -> auth` edge | The gateway's default steps; its auth call runs the route's body, **once** even when the edge has `calls: N` (N checks model ordinary routes). |
| Gateway without the edge              | The gateway's steps, then the body inline: no network hop, auth's own `proc` once.                                                             |
| No gateway (or an auth node alone)    | The body alone.                                                                                                                                |

The login body is auth's `proc`, then its callees, caches (reads) and databases, like a service. JWKS is auth's `proc` only. A custom `chain:` on either route runs with the auth node as runner. A spec route with the same key replaces the built-in route and adds a warning. The HEAD copy of the JWKS keeps its `responder`, and an explicit `HEAD /.well-known/jwks.json` entry is treated like one for a spec `GET`: the auth node owns it and it keeps the `Jwks` responder. `skeleton` never lists them: they are not spec routes. Ordinary routes keep the gateway's `auth_request` call with auth's `proc` only.

## Skeletons

`skeleton(topo, routes, existing)` returns the lines `kaanal endpoints` prints: one `"KEY": {} # owner NODE, op OP` per spec route that has no entry, sorted. `existing` is a list of keys, not the entry map, so the binary needs no `indexmap` dependency. Keys match after `normalize` (method upper-cased, path trimmed); the key is written as a JSON string, which is a valid YAML double-quoted scalar. HEAD copies are never listed.

## HEAD copies

Every `GET` endpoint without a `HEAD` gets a `HEAD` copy with its own key (own draws), flagged `synthesized` and not listed. An explicit `HEAD` entry for a spec `GET` route uses that route's owner and op.
