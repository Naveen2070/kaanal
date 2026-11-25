# `crates/kaanal-server/src/log_api.rs`

The log control routes: `GET /__kaanal/logs` and `GET /__kaanal/trace/{id}`. Both sit behind the control guard (token, `Host`, no CORS) like every other control route, and both read the ring under its lock only long enough to clone what they return. Neither writes anything, so neither goes through `lock` / `commit` / `emit` of [control.md](control.md). Reference for users: [control-api guide](https://kaanal.naveenr.in/docs/reference/control-api/).

## Handlers

| Handler | Route                      | What it does                                                                                                                                                                                                                                                                                                       |
| ------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `logs`  | `GET /__kaanal/logs`       | Parses the query as ordered pairs (so a repeated key is seen), takes `limit` (`limit_of`: plain digits in 1 to 1000, default 100), builds the `Filter` from the pairs with the current time (`since`), and answers `LogRing::query` as `{ entries, oldest, newest, cap, evicted }`. Oldest completion first.       |
| `trace` | `GET /__kaanal/trace/{id}` | `LogRing::find` on the percent-decoded path segment: `200` with the entry, `404 { "error": "evicted", "detail": "older than the last 1000 requests, or still in flight" }`, `404 { "error": "unknown" }`, or `400` for a key that is neither an id, `last` nor a valid client id (or whose decoding is not UTF-8). |

Errors use the control API's `{ "error": ... }` shape; `evicted` adds `detail`, which `kaanal` appends in parentheses.

## Rules worth knowing

- Nothing is applied partly: `Filter::from_params` rejects an unknown, repeated, empty (except `scope`) or malformed parameter, naming it; `limit` is checked here, not there. `limit=+5`, `limit= 5`, `limit=0`, `limit=1001` and a number too large for a `usize` are all `400`.
- `after=` is the polling cursor: pass the highest `done` seen. With `after`, the answer is the **first** `limit` entries after the cursor (not the newest), so a poller that falls behind pages forward and skips nothing. The page carries `boot`, this start's nonce: `done` counts from 1 again after a restart, so a poller starts over when `boot` changes. Because entries are ordered by completion, nothing is missed, however out of order requests finish.
- A bad query string (not decodable) is the `400` of axum's own rejection, with its message.
- The ring and the bus are fed in this order: entry first, event second (`mock::publish`). So when an SSE event for id X is received, `GET /trace/X` finds it. `kaanal logs -f` relies on this.

## Tests

[../../tests.md](../../tests.md#crateskaanal-servertestslogs_httprs--ids-events-logs-and-trace): `tests/logs_http.rs` (the second half: the log routes).
