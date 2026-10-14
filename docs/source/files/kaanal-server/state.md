# `crates/kaanal-server/src/state.rs`

`AppState`: everything the running server knows, built once at startup.

## What it does

`AppState::build` is the full load pipeline shared by `serve` and `check` (`kaanal validate`): topology → OpenAPI specs → endpoints (inline map or the endpoints file) compiled by `kaanal_core::chain::compile` → route table. Every problem is a fatal `Located` error (a path into `kaanal.yaml`, or a place in the endpoints file, plus a precise message) or, for ignorable schema keywords, a collected warning.

## Types

| Item       | Kind   | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Route`    | struct | One `method + path`: `key` (`"GET /users/{id}"`, also the chain seed material), `endpoint` (the compiled `Arc<Endpoint>`: chain, op, `override`, `error_map`, HEAD copies included), `status` (success status from the spec, 200 for a static route; an `override` status wins), `schema` (response schema for generated bodies), `counter` (atomic count → `ChainRequest.n`).                                                                                                                                                                                                                                                  |
| `Event`    | struct | One finished request for the SSE stream: `ts_ms`, `method`, `path`, `status`, measured `ms`, `nodes` visited in order, `failed_node`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `AppState` | struct | `topo`, `paths` (matchit router of `Vec<Route>` per path), `route_keys` (sorted, for display; synthesized HEAD routes excluded), `endpoints` (the compiled `Endpoints`, synthesized HEAD included: the control API looks keys and `entry_node` up here), `chaos` (`RwLock<ChaosState>`), `events` (broadcast sender, capacity 1024), `token`, `seed`, `chain_seed`, `drawn_seed`, `port`, `warnings`, `auth` (the token `Issuer`; set by `serve`, never by `check`, so `validate` needs no key), `engine` (`Option<EngineHandle>`, the live replay engine; `None` after `build`, set by `serve`, since `check` has no runtime). |

**Auth warning.** An auth node with no `users` adds `node 'login': no users, so every POST /login answers 401 (add users: [{ username, password }])` to `warnings`.

## Seeds

`seed` is the config's `seed:`: generated bodies always use it, in both determinism modes (D14). `chain_seed` seeds the chain draws: `seed:` under `determinism: controlled`, a value drawn from OS entropy once at startup under `realistic` (the server draws it, so `kaanal-core` stays free of entropy). `drawn_seed` is `Some(that value)` under `realistic` and `None` otherwise; `up` prints it so the run can be replayed as `controlled`.

## `AppState::build` pipeline

1. **Topology** — `Topology::build(cfg)`; any `TopologyError` aborts. (`concurrency`, pools and rate limits no longer warn: live mode enforces them through the engine actor.)
2. **Specs** (`load_specs`) — for each service or monolith node with `spec`, reads the file (relative to the config's directory) and parses `ApiModel::from_openapi`. Warnings (ignored keywords) are collected, prefixed with the file name. Each endpoint becomes a `RouteSpec` plus its success status and schema. Two services defining the same `method + path` → error naming both owners (so `compile` always gets unique keys). `check_shape` runs on every path.
3. **Compile** (`compile_endpoints`) — `compile(topo, route specs, entries)`. Entries come from the inline `endpoints:` map, or from the file it names (relative to the config's directory), read and parsed here (`parse_endpoints`). Compile warnings (stale entries, ignored `via` error maps) join `warnings`.
4. **Routes** — one `Route` per compiled endpoint, in key order. A static endpoint (no owner) has its path shape-checked against the spec paths, so `/users/{uid}` next to a spec `/users/{id}` and the `/__kaanal` prefix are still rejected. A `HEAD` without a spec entry of its own takes GET's status and schema (R6.8). HEAD copies are served but not listed in `route_keys`.
5. **Router** — paths go into `matchit::Router` in `BTreeMap` order, so a conflict always names the same path deterministically.
6. **Seed** — `cfg.seed_source()`: `Fixed` → no draw; `Entropy` → one `rand::random::<u64>()`.

## Error places

`build` returns `Located`: topology errors use `TopologyError::located`; spec file and route-conflict errors use `nodes.<n>.spec`; a missing endpoints file is placed at the `endpoints:` value of `kaanal.yaml`; a parse or compile error inside the endpoints file carries that file (`Located.file`, plus `pos` for a parse error), so `kaanal` prints `kaanal.endpoints.yaml:line:col: ...`; a compile error from an inline entry uses `endpoints.<key>...`; a static route's shape error is placed at its entry; the router insert has an empty path (no position). A spec node with no edge from the gateway is rejected by the topology check before `compile` could meet it (a spec route without an entry has no place of its own).

## `check_shape`

Rejects two paths that differ only in parameter names (`/users/{id}` vs `/users/{uid}`) — matchit would fail with an unhelpful message, so this produces a clear one naming both spellings. Also reserves the `/__kaanal` namespace: a mock there would be served without CORS next to the control API (rule R5.4). `compile` checks neither, so the server runs it on every spec path and every static endpoint.

## Tests

[../tests.md](../../tests.md#crateskaanal-servertestsstaters) — conflict detection, endpoints file errors, warnings plumbing.
