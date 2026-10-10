# `crates/kaanal-server/src/lib.rs`

Server entry: binds the port, builds state, installs the router.

## What it does

The only public surface the binary uses. One axum router serves **both** the control API and the mocks on one port (spec FR-API-12). Control routes are mounted first and wrapped in the `guard` middleware via `route_layer`, so the mock fallback can never accidentally bypass authentication.

## Items

| Item                                                         | Kind   | What it does                                                                                                                                                                                                                                                                                                          |
| ------------------------------------------------------------ | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ServeOpts`                                                  | struct | `host`, `port` (0 = OS-chosen, used by tests), `token`.                                                                                                                                                                                                                                                               |
| `BoundServer`                                                | struct | `addr` (actual bound address), `warnings` (non-fatal load findings), `drawn_seed` (the chain seed drawn at startup under `determinism: realistic`, else `None`; print it so the run can be replayed as `controlled`), private `handle` (the spawned serve task).                                                      |
| `BoundServer::wait()`                                        | fn     | Awaits the serve task; returns when the server stops.                                                                                                                                                                                                                                                                 |
| `check(cfg, base_dir) -> Result<Vec<String>, Located>`       | fn     | Full dry-run: builds `AppState` (config, topology, spec files, endpoints inline or from the endpoints file, route conflicts) without binding a port. Backs `kaanal validate`. Returns warnings on success; errors carry their place in `kaanal.yaml` or in the endpoints file (`Located::in_source`, `Located.file`). |
| `new_token() -> String`                                      | fn     | 32 random bytes as 64 hex chars. New on every `kaanal up`.                                                                                                                                                                                                                                                            |
| `serve(cfg, base_dir, opts) -> Result<BoundServer, Located>` | fn     | Binds the listener, builds `AppState`, spawns `axum::serve` on the tokio runtime, returns the handle.                                                                                                                                                                                                                 |
| `bind(host, port)`                                           | fn     | Tries the requested port, then the next 20, before giving up with a message listing the range.                                                                                                                                                                                                                        |
| `router(st)`                                                 | fn     | Builds the route table (below).                                                                                                                                                                                                                                                                                       |

## Route table

| Route                               | Handler                             | Notes                                                                     |
| ----------------------------------- | ----------------------------------- | ------------------------------------------------------------------------- |
| `GET /__kaanal/health`              | inline                              | `{ "ok": true }`; no token needed (liveness probe)                        |
| `GET /__kaanal/topology`            | `control::topology`                 | nodes, edges, route keys                                                  |
| `GET/DELETE /__kaanal/chaos`        | `control::get_chaos` / `clear_all`  | list / wipe all chaos                                                     |
| `PUT/DELETE /__kaanal/chaos/{node}` | `control::set_chaos` / `clear_node` | merge a patch / clear one node                                            |
| `GET /__kaanal/events`              | `control::events`                   | SSE stream of request events                                              |
| `GET /__kaanal/ui`                  | `control::ui`                       | embedded HTML page; no token needed (page reads it from the URL fragment) |
| _(fallback)_                        | `mock::handle`                      | every other path → mock engine                                            |

The `route_layer(guard)` covers exactly the routes above it in the builder — a new control route added there cannot forget authentication.
