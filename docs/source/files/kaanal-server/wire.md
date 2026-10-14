# `crates/kaanal-server/src/wire.rs`

The body stream behind the `reset` failure kind (B2a). Full behaviour: [../../chaos.md](../../chaos.md#failure-kinds).

## What it does

`Cut` is a `Stream<Item = Result<Bytes, io::Error>>` used as the response body when `reset` hits a wire node. The client must see the status line and part of the body before the connection aborts. The plan-task-0 spike found that hyper 1.x only flushes the head when the stream is shaped as **one chunk, then one `Poll::Pending` (waker woken), then the error**: an error on the first poll, or straight after the chunk, drops the head and the client gets an empty reply (reqwest: `IncompleteMessage`, which a browser may silently retry on a kept-alive connection, re-rolling chaos).

## Items

| Item          | Kind   | What it does                                                                                                                                                                          |
| ------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Cut`         | struct | Holds the chunk and a `parked` flag. `poll_next`: first call yields the chunk; second returns `Pending` after waking the waker; third yields a `ConnectionReset` error.               |
| `Cut::new(b)` | fn     | A stream that delivers `b` and then breaks. The caller sets `Content-Length` to the full body length (at least chunk + 1), so the client reads part of the body, then sees the abort. |

This is not a literal TCP RST (R1.18). No unit tests here; the raw-socket check is `reset_on_the_owner_behind_a_gateway_aborts_the_connection` in [../../tests.md](../../tests.md#crateskaanal-servertestshttprs).
