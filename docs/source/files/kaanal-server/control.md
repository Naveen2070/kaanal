# `crates/kaanal-server/src/control.rs`

The control API handlers and the security guard that stands in front of them.

## The guard (runs before every `/__kaanal/*` route)

`guard(State, req, next)` — a middleware, so it runs **before any extractor parses a body or query**; unauthenticated callers never reach handler code:

1. **Host check** — `host_allowed(host, port)`: must be `localhost`, `127.0.0.1` or `[::1]`, with no port or the bound port (DNS-rebinding guard, rule R5.3). Case-insensitive; IPv6 bracket parsing; rejects tricks like `localhost.evil.example` and `evil.example:8080@localhost`. Failure → 403.
2. **Token check** — every path except `/__kaanal/health` and `/__kaanal/ui` needs `Authorization: Bearer <token>`. `/__kaanal/events` alone may take `?token=` because the browser `EventSource` API cannot set headers (rule R5.2). Comparison is `ct_eq` — constant-time, so the token can't be guessed byte-by-byte from response timing. Failure → 401.
3. **Response hardening** — `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every control response.

## Handlers

| Handler      | Route                           | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------ | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `topology`   | `GET /__kaanal/topology`        | JSON dump: nodes (name/type/flavor), edges (from/to names), sorted route keys.                                                                                                                                                                                                                                                                                                                                                                                  |
| `get_chaos`  | `GET /__kaanal/chaos`           | Map of node name → `ChaosView` for every node with chaos set.                                                                                                                                                                                                                                                                                                                                                                                                   |
| `set_chaos`  | `PUT /__kaanal/chaos/{node}`    | **Merges** a `ChaosPatch` into the node's existing settings (absent fields keep their values). The merged copy is checked by core's `NodeChaos::validate` (rule R6.9: `error_rate` 0–1, `error_status` 500–504, `latency_ms` ≤ 300 000 — 5 min, `MAX_LATENCY_ADD`) and stored only if valid; a bad patch → 400 and nothing changes. Unknown node → 404; bad body → the extractor's own 4xx. `deny_unknown_fields` rejects typo'd keys. Returns the merged view. |
| `clear_node` | `DELETE /__kaanal/chaos/{node}` | Removes one node's chaos → 204. Unknown node → 404.                                                                                                                                                                                                                                                                                                                                                                                                             |
| `clear_all`  | `DELETE /__kaanal/chaos`        | Wipes all chaos → 204.                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `events`     | `GET /__kaanal/events`          | SSE stream: every request event as JSON (`event: request`), plus `event: lagged` if a slow subscriber drops messages. Backed by the `broadcast` bus in `AppState`; keep-alive enabled.                                                                                                                                                                                                                                                                          |
| `ui`         | `GET /__kaanal/ui`              | The embedded HTML page (`include_str!`). Served with `Referrer-Policy: no-referrer` and a strict CSP (inline script/style only, `connect-src 'self'`, `frame-ancestors 'none'`). The page reads the token from the URL **fragment**, which browsers never send to a server.                                                                                                                                                                                     |

## Supporting items

| Item                             | What it does                                                                         |
| -------------------------------- | ------------------------------------------------------------------------------------ |
| `reject(code, msg)`              | Builds the uniform `{ "error": msg }` JSON error tuple used by all handlers.         |
| `query_token(query)`             | Extracts `token=` from a query string; `xtoken=` or a bare `token` do not match.     |
| `ChaosView` / `From<&NodeChaos>` | The public JSON shape of chaos (`latency_ms`, `error_rate`, `error_status`, `down`). |
| `ChaosPatch`                     | The accepted PUT body — all fields optional, unknown fields denied.                  |

## Unit tests in this file

`hosts` (Host parsing incl. attack spellings), `constant_time_eq`, `token_comes_only_from_the_token_parameter`. HTTP-level tests: [../tests.md](../../tests.md#crateskaanal-servertestshttprs).
