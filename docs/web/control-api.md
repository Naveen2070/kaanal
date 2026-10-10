# Control API reference

Everything under `/__kaanal/` — the single control plane used by the CLI, the status page, and your own scripts and test setups.

## Authentication & transport rules

- **Token**: every endpoint except `health` and `ui` requires `Authorization: Bearer <token>`. The token is printed by `kaanal up` and stored in `.kaanal/run.json`. On `/__kaanal/events` only, `?token=` is accepted (browser `EventSource` can't set headers).
- **Host guard**: the `Host` header must be `localhost`, `127.0.0.1` or `[::1]`, with the bound port (or none). Anything else → `403`. This blocks DNS-rebinding attacks from browsers.
- **No CORS** on this namespace, ever — browsers on other origins cannot call it.
- Responses carry `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.
- Errors are always JSON: `{ "error": "message" }`.

## Endpoints

### `GET /__kaanal/health`

Liveness probe. No token.

```json
{ "ok": true }
```

### `GET /__kaanal/topology`

The running topology.

```json
{
  "nodes": [ { "name": "nginx", "type": "gateway", "flavor": "nginx" } ],
  "edges": [ { "from": "nginx", "to": "app" } ],
  "routes": [ "GET /health", "GET /users", "GET /users/{id}", "POST /orders" ]
}
```

### `GET /__kaanal/chaos`

All active chaos, keyed by node name. Empty object when nothing is set.

```json
{
  "db": { "latency_ms": 300, "error_rate": 0.1, "error_status": 503, "down": false }
}
```

### `PUT /__kaanal/chaos/{node}`

**Merges** a patch into the node's chaos — fields you omit keep their current values. Returns the merged state.

```json
{ "latency_ms": 300, "error_rate": 0.1, "error_status": 504, "down": true }
```

| Field          | Rules                                                      |
| -------------- | ---------------------------------------------------------- |
| `latency_ms`   | integer, 0 – 300 000                                       |
| `error_rate`   | number, 0.0 – 1.0                                          |
| `error_status` | integer, 500 – 504                                         |
| `down`         | boolean — when true, everything through the node fails 503 |

Unknown node → `404 "unknown node '<name>'"`. Out-of-range values → `400`. Unknown JSON keys → rejected.

### `DELETE /__kaanal/chaos/{node}` / `DELETE /__kaanal/chaos`

Clear one node (`404` if unknown) or all chaos. Both return `204`.

### `GET /__kaanal/events`

Server-Sent Events stream of every mock request as it completes.

```
event: request
data: {"ts_ms":1759900000000,"method":"GET","path":"/users/7","status":200,"ms":12,"nodes":["nginx","app","db"],"failed_node":null}

event: lagged
data: 37
```

- `ms` is measured wall time; the per-hop _simulated_ breakdown is in the response's `X-Kaanal-Trace` header (parallel branches in `[a|b]`, cache lookups as `hit`/`miss`, long chains ending `,...+N hops`).
- `lagged` means the stream fell behind and skipped that many events.

### `GET /__kaanal/ui`

The embedded status page (HTML). No token in the request — the page reads it from the URL fragment (`#token=…`), which browsers never send to servers. See [web-ui.md](web-ui.md).

## Example: drive chaos from a test setup

```bash
TOKEN=$(jq -r .token .kaanal/run.json)
PORT=$(jq -r .port .kaanal/run.json)
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
     -d '{"down": true}' "http://127.0.0.1:$PORT/__kaanal/chaos/db"
# ... run the test ...
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/chaos"
```
