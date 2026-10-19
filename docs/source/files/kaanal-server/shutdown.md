# `crates/kaanal-server/src/shutdown.rs`

The graceful stop: one `Shutdown` handle per server, asked by `POST /__kaanal/shutdown` or `BoundServer::shutdown`, and the listener wrapper that makes the deadline real.

## What it does

The first request fixes a deadline (`now + budget`); later requests never move it. Until the deadline the server **drains**: `axum::serve(..).with_graceful_shutdown(requested())` stops accepting, and in-flight requests finish. At the deadline every connection still open is **cut**.

Cutting needs a trick. axum spawns each accepted connection on its own task, so aborting the serve task (what `BoundServer::wait` does at the deadline) would leave those tasks running and their clients attached. Instead the listener is wrapped in `Cut`, whose connections fail every read and write with `ConnectionAborted` once the deadline has passed. hyper treats that like a client disconnect: the handler future is dropped, and the request log's drop guard records `client-gone` (the process is about to exit, so nothing keeps that entry). User-facing description: [../../../web/lifecycle.md](../../../web/lifecycle.md#the-drain).

## Items

| Item                        | Kind       | What it does                                                                                                                                                                                                                          |
| --------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Shutdown`                  | struct     | A `watch` channel holding `Option<Instant>`: `None` until someone asks, then the fixed deadline. `Clone`; clones share the state. Lives in `AppState.shutdown`.                                                                       |
| `Shutdown::new()`           | fn         | No shutdown requested.                                                                                                                                                                                                                |
| `Shutdown::request(budget)` | fn         | Fixes the deadline on the first call (inside `send_if_modified`, so two racing calls agree) and returns `budget`. Later calls return the time left and change nothing.                                                                |
| `Shutdown::remaining()`     | fn         | Time left, zero once past, `None` before any request. Used for the `202` body of a repeated call and the `timeout_ms` of the `shutdown` event.                                                                                        |
| `Shutdown::watch()`         | fn         | A receiver that sees the deadline appear. `events` merges it into its stream so the stream ends on a request.                                                                                                                         |
| `Shutdown::requested()`     | async fn   | Resolves once a request exists. Wired to `with_graceful_shutdown`, and ends `lease_task` (no scenario lease should fire during the drain).                                                                                            |
| `Shutdown::expired()`       | async fn   | Resolves when the deadline passes, never if nothing is requested. `BoundServer::wait` selects on it next to the serve task, then aborts that task and returns `Ok`.                                                                   |
| `Cut`, `Cut::new(tcp, sd)`  | struct, fn | An `axum::serve::Listener` over a `TcpListener`. Each accepted stream becomes a `CutIo` that holds a pinned `expired()` future.                                                                                                       |
| `CutIo`                     | struct     | One connection. `poll_read`, `poll_write` and `poll_flush` first poll the `expired()` future (which also registers the waker, so an idle connection wakes at the deadline) and fail once it is ready; `poll_shutdown` passes through. |

## Behaviour to know

- A tiny budget (a few milliseconds) can cut the connection that carried the shutdown request before its `202` is flushed: the deadline is fixed before the handler answers. `kaanal down` reads a transport error with the instance lock held as "the drain is running". Documented on [`control::shutdown`](control.md) and in [control-api.md](../../../web/control-api.md#post-__kaanalshutdown).
- `/__kaanal/events` streams end at the request with a final `shutdown` message instead of holding the drain for the whole budget; the events of the requests that are cut therefore reach nobody.
- The deadline uses `tokio::time::Instant`, so the unit tests run on the paused clock.

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcshutdownrs-unit--the-deadline-handle) for the unit tests, [../../tests.md](../../tests.md#crateskaanal-servertestsshutdown_httprs--the-drain-over-http) for the HTTP ones.
