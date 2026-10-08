# The embedded status page (`/__kaanal/ui`)

A single self-contained HTML page served by the binary — no build step, no external assets, no frameworks. Open the full URL printed by `kaanal up`:

```
http://localhost:8080/__kaanal/ui#token=<64 hex chars>
```

The token rides in the URL **fragment** (`#…`), which browsers never send to any server; the page reads it with `location.hash` and uses it for its API calls.

## What it shows

**Node table** — one row per topology node:

| Column        | Meaning                                                                  |
| ------------- | ------------------------------------------------------------------------ |
| Node / Type   | Name, kind and flavor (`database · postgres`).                           |
| rps / err %   | Rolling 10-second window, recomputed every second from the event stream. |
| Latency +ms   | Extra latency input (0–300 000).                                         |
| Error %       | Failure probability input (0–100).                                       |
| Status        | Failure status select (500–504).                                         |
| Down          | Hard-outage checkbox.                                                    |
| Apply / Clear | PUTs the merged chaos patch for that row, or DELETEs it.                 |

**Request log** — newest first, capped at 200 rows: time, method, path, status, ms (measured), and the node that failed (rows with 5xx are highlighted).

Two message lines sit at the top: one for API errors (role `alert`), one for stream status (role `status`), so one never hides the other.

## Behaviour details

- **Live updates** come from `GET /__kaanal/events` (SSE) using `?token=` — the one endpoint where a query token is allowed.
- **Reconnect handling**: if the stream drops, the page retries and reloads the node table on reconnect (chaos may have changed meanwhile). If Kaanal restarted (new token), it tells you to open the new link.
- **Lagged stream**: if events arrive faster than the page can display, it says how many were skipped instead of falling behind silently.
- **Light/dark**: follows the OS via `color-scheme`.
- **Accessibility**: every chaos control has an aria-label naming its node (table headers aren't associated with inputs).

## Security rules the page follows

These are enforced by static tests in `crates/kaanal-server/tests/http.rs`:

- Every dynamic value (paths, node names, API errors) goes through `textContent` — never `innerHTML` or friends.
- No `eval`, no inline event handlers, no `localStorage`, no external URLs.
- The token exists only in a JS variable: sent as a header on API calls and as `?token=` on the event stream. It is never stored or logged.
- Served with a strict CSP (inline script/style only, `connect-src 'self'`, `frame-ancestors 'none'`) and `Referrer-Policy: no-referrer`.
- Node names are used as `Map` keys, never plain object keys (a node literally named `__proto__` can't break the page).
- The rps/error% window is fixed-size counter buckets — memory stays flat no matter how fast requests arrive.

## Known limitations

- Chaos set from the **CLI** shows in the page only after a Clear or a stream reconnect — there is no polling of `/__kaanal/chaos`.
- Each open tab holds one SSE connection; browsers allow ~6 connections per host over HTTP/1.1, so many tabs can stall other requests to the port. Close extra tabs.

> Contributor pointer: the page source is `crates/kaanal-server/src/ui.html`, served by `control::ui` with its CSP headers — see [../source/files/kaanal-server/control.md](../source/files/kaanal-server/control.md).
