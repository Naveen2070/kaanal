---
title: "Logs and trace"
sidebar:
  order: 7
---

## `GET /__kaanal/logs`

The request log: the last finished mock requests, oldest completion first. What an entry holds, the flags and the commands that use this route are in [observability](/docs/guides/observe/observability/).

| Parameter  | Meaning                                                                                                                                                                                                     |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `limit`    | How many of the latest matching entries to return: an integer from 1 to 1000. Default 100.                                                                                                                  |
| `endpoint` | The route template as logged, `GET /users/{id}` (the method's case and the spacing are normalised, the text must be `METHOD /path`). A concrete URL such as `GET /users/1` is accepted and matches nothing. |
| `status`   | Comma separated codes and classes: `404`, `5xx`, `404,5xx`.                                                                                                                                                 |
| `failed`   | `true` keeps requests with a `failed_node`; `false` keeps everything.                                                                                                                                       |
| `scope`    | The exact scope key as echoed in `X-Kaanal-Scope`. **`scope=` with no value** is the scope of an empty `X-Kaanal-Scenario` header, not "any scope".                                                         |
| `since`    | A duration (`5m`, `30s`) or an RFC 3339 time: only requests that finished at or after it.                                                                                                                   |
| `after`    | A `done` number: only entries that finished after it. To poll, pass the highest `done` you have seen.                                                                                                       |

```json
{
  "entries": [
    {
      "id": "r19a2-0004", "client_id": null, "seq": 4, "done": 4, "ts_ms": 1792345019872,
      "method": "GET", "path": "/users/7", "endpoint": "GET /users/{id}", "status": 400,
      "ms": 0, "trace_ms": 0.0, "scope": null, "pin": null, "flags": [],
      "chain_error": null, "failed_node": null, "kind": null, "rejected": "bad-scope",
      "trace": "", "hops": [], "truncated": false
    }
  ],
  "oldest": "r19a2-0001",
  "newest": "r19a2-0004",
  "cap": 1000,
  "evicted": 0
}
```

- `entries` are ordered by completion (`done`), not by id: a slow request that started first comes after the fast ones that overtook it.
- `oldest` and `newest` are the ids with the lowest and highest number **in the ring** (both `null` when it is empty). `cap` is the ring size, 1000, and `evicted` counts the entries pushed out since start (a `POST /reset` does not change it).
- A hop is `{ node, replica?, kind, op?, start_ms, dur_ms, wait_ms, lane, fork, cut, why }`; `kind` is `net`, `proc`, `op` (then `op` is `read`, `write` or `scan`), `hit`, `miss` or `delay`. `replica` (0-based) is present only for a node with more than one replica, and the trace text names that hop `db#2`. `chain_error` is `{ node, status, kind, cause }`.
- Nothing is applied partly: an unknown, repeated, empty (except `scope`) or malformed parameter answers `400 { "error": "parameter 'limit': expected an integer from 1 to 1000" }`, naming the parameter. `limit` takes plain digits only (no sign, no space).
- `POST /__kaanal/reset` empties the ring. Request ids keep counting.

## `GET /__kaanal/trace/{id}`

One log entry, in the same shape as above. `{id}` is one of:

- Kaanal's id, `r19a2-0004`. Leading zeros do not matter (`r19a2-00004` is the same entry).
- `last`: the most recent entry **to finish**.
- A client `X-Request-Id` that was sent with a request (the newest request that used it).

A text that is also an id is read as an id first. Percent-escapes are decoded.

| Status | Body                                                                                        | When                                                                                                            |
| ------ | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `200`  | the entry                                                                                   | Found.                                                                                                          |
| `404`  | `{ "error": "evicted", "detail": "older than the last 1000 requests, or still in flight" }` | An id of this run, at or below the highest number recorded, that is not in the ring.                            |
| `404`  | `{ "error": "unknown" }`                                                                    | An id of another run, a number above the highest recorded, a client id nobody sent, or `last` on an empty ring. |
| `400`  | `{ "error": "expected a request id like r04f2-0042, 'last' or a client X-Request-Id" }`     | Neither an id, `last` nor a valid client id (`[A-Za-z0-9._-]`, at most 64), or escapes that are not UTF-8.      |

Both routes need the token and the `Host` guard like every other control route, and neither is open to CORS.
