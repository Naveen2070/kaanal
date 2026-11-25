---
title: "Shutdown and the status page"
sidebar:
  order: 9
---

## `POST /__kaanal/shutdown`

Stops the instance gracefully: it stops accepting connections, lets requests already in flight finish for the budget, cuts what is left, removes `.kaanal/run.json` and exits 0. `kaanal down`, `SIGTERM` and Ctrl+C take the same path; see [lifecycle](/docs/guides/observe/lifecycle/) for the whole story.

The body is optional. A missing, empty or whitespace-only body means a 5 s budget:

```json
{ "timeout": "30s" }
```

`timeout` is a duration string (`500ms`, `5s`, `1m`), 1 ms to 300 s. It is the only key.

```http
HTTP/1.1 202 Accepted
content-type: application/json

{"shutting_down":true,"timeout_ms":30000}
```

The first call fixes the deadline. A second call that still reaches the server (on a connection that was already open; new connections are refused once the drain starts) changes nothing: it answers `202` with `timeout_ms` set to the time left, and never extends the deadline.

| Status | When                                                                                                                         |
| ------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `202`  | The drain has started (or was already running).                                                                              |
| `400`  | The body is not a JSON object, has a key other than `timeout`, or `timeout` is not a duration string between 1 ms and 300 s. |
| `401`  | Missing or wrong token.                                                                                                      |
| `403`  | A `Host` header that is not `localhost`, `127.0.0.1` or `[::1]`.                                                             |
| `405`  | Any method other than `POST`.                                                                                                |

A `400` stops nothing: the server keeps running.

**A tiny budget can lose the `202`.** The deadline is fixed before the answer is sent, so with a budget of a few milliseconds the connection can be cut before the `202` reaches you. A client that gets a transport error from this call has no way to tell that from a server that was already gone; `kaanal down` reads it as "the drain is running" because it still holds the instance lock, and waits for the process to end. Do the same: check that the process is gone rather than retrying the call.

Every open [`/__kaanal/events`](/docs/reference/control-api/live-and-events/#get-__kaanalevents) stream ends at once with a final `shutdown` message, so a status page or a follower never holds the drain open.

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -d '{"timeout":"10s"}' "http://127.0.0.1:$PORT/__kaanal/shutdown"
```

## `GET /__kaanal/ui`

The embedded status page (HTML): a health dashboard that needs no token and makes no call but `GET /__kaanal/health`. A token in the URL fragment (`#token=…`, which browsers never send to servers) is only carried onto its "Open Studio" link. `kaanal up` prints the Studio link, not this one. See [the status page](/docs/guides/observe/status-page/).
