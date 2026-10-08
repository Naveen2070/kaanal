# `crates/kaanal-server/src/state.rs`

`AppState`: everything the running server knows, built once at startup.

## What it does

`AppState::build` is the full load pipeline shared by `serve` and `check` (`kaanal validate`): topology → OpenAPI specs → route table → overrides. Every problem is a fatal error with a precise message (file paths included) or, for ignorable schema keywords, a collected warning.

## Types

| Item       | Kind   | What it does                                                                                                                                                                                                                                     |
| ---------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Body`     | enum   | `Generated { schema }` (from an OpenAPI response) or `Static(Value)` (from a `routes:` override).                                                                                                                                                |
| `Route`    | struct | One `method + path`: `key` (`"GET /users/{id}"` — also the chain RNG key), `owner` (`None` = static-only route answered by the gateway), `op` (inferred read/write/scan), `status`, `body`, `counter` (atomic request count → `ChainRequest.n`). |
| `Event`    | struct | One finished request for the SSE stream: `ts_ms`, `method`, `path`, `status`, measured `ms`, `nodes` visited in order, `failed_node`.                                                                                                            |
| `AppState` | struct | `topo`, `paths` (matchit router of `Vec<Route>` per path), `route_keys` (sorted, for display), `chaos` (`RwLock<ChaosState>`), `events` (broadcast sender, capacity 1024), `token`, `seed`, `port`, `warnings`.                                  |

## `AppState::build` pipeline

1. **Topology** — `Topology::build(cfg)`; any `TopologyError` aborts. Each node with `concurrency` adds a warning (the simulator honours it, live mode does not yet; M1).
2. **Specs** — for each service node with `spec`, reads the file (relative to the config's directory) and parses `ApiModel::from_openapi`. Warnings (ignored keywords) are collected, prefixed with the file name.
3. **Spec routes** — each endpoint becomes a `Route`. Two services defining the same `method + path` → error naming both owners. `check_shape` (below) runs on every path.
4. **Overrides** — each `routes:` entry is validated (`"METHOD /path"` shape, known method, leading slash, status 200–599; 1xx is not a final response) and either **replaces** an existing spec route's status/body (same shape, same param names) or **adds** a static-only route.
5. **Router** — paths go into `matchit::Router` in `BTreeMap` order, so a conflict always names the same path deterministically.

## `check_shape`

Rejects two paths that differ only in parameter names (`/users/{id}` vs `/users/{uid}`) — matchit would fail with an unhelpful message, so this produces a clear one naming both spellings. Also reserves the `/__kaanal` namespace: a mock there would be served without CORS next to the control API (rule R5.4).

## Tests

[../tests.md](../../tests.md#crateskaanal-servertestsstaters) — conflict detection, override rules, warnings plumbing.
