# Admin mode: your frontend in the live numbers, and rewriting a response

Studio has three modes beside the Apply button: **Live**, **Batch** and **Admin**. Live runs traffic Kaanal makes up. Batch runs once, seeded and repeatable. **Admin is the live view with your own frontend in it**: point the frontend (or `curl`, or a test) at the mock and its requests appear in the numbers next to the generated ones. It also lets you rewrite what one route answers while the frontend is calling it.

Nothing needs configuring. The frontend keeps using the mock's port, as it does anywhere else.

## What you see

- **The strip** is the live strip (arriving, dropped, p50 and p99, busiest component) plus a line that splits the traffic: `Real 12 / s`, `Generated 80 / s`. Until a real request arrives it says so. **Real only** turns every figure on the strip and on the canvas cards to the real requests alone; the queues and slots in use stay the system's, because the real and the generated requests share them.
- **The canvas** draws a real request as a **ring** and a generated one as a dot, so the two differ by shape, never only by colour.
- **The Log tab** becomes the **request feed**: what the mock answered, newest first, with the status each request got. It is polled once a second while the tab is open and the page is visible. A request an edit answered says **edited** in words.

A real request goes through the same simulated system as a generated one. It waits in the same queues, takes the same time at each component and meets the same chaos, so a frontend sees what a user would see when the system is loaded.

## Rewriting what a route answers

Press **Edit response** on a row of the feed. The editor opens on what the route answers now (or on the edit it already has). Set the **status**, any **headers** and the **JSON body**, then:

| Button                | What it does                                                                                                                                                         |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Apply now**         | From the next request on, the route answers exactly this. Nothing to restart. It is kept in memory and marked _Live only, not in the file_.                          |
| **Back to normal**    | Removes the edit.                                                                                                                                                    |
| **Stage in the file** | Writes it into the draft as the route's `override:` (a `set` or `insert` in `kaanal.yaml`, in the file's own key spelling). The usual **Save** then writes the file. |

A problem is said under the field it is about: a status outside 200 to 599, a body that is not JSON, a body on a `204` or `304`, a header Kaanal sets itself (`content-length`, `content-type`, `content-encoding`, `access-control-*`, `x-request-id`, `x-kaanal-*` and the connection headers), a header value over 4096 bytes, a body over 1 MiB, more than 32 headers.

### Who wins

A route answers, from the top:

1. **Chaos** (a node that is down, an error rate, a broken connection). A failing route keeps failing under your edit, so you can test failures with it in place.
2. **A pin** that forces `error` or `empty`. A pin of `loading`, `slow` or `success` lets the edit through.
3. **Your edit.**
4. The route's `override:` in the file.
5. The generated body.

Three things never take an edit: a route that reads a [collection](collections.md) (an edit would freeze live data), the built-in login and key-set routes of an [auth](auth.md) component, and a `HEAD` made from its `GET` (edit the `GET`; the `HEAD` follows).

A request that names a [scenario scope](scenarios.md) (`X-Kaanal-Scenario`) never sees an edit, the same as it never sees live pins, so parallel test workers stay apart.

### How long an edit lives

- **Parameter edits** in Studio keep your edits. An **Apply** keeps the edits whose route still takes one and drops the rest, and says which in the reply (`payloads_dropped`).
- **`POST /__kaanal/reset`** clears every edit.
- A restart of the server loses edits that were not staged and saved.
- If `endpoints:` in your `kaanal.yaml` names another file, **Stage in the file** is off (Studio does not edit that file); **Apply now** still works.

A header an edit (or an `override:`) sets is listed in `Access-Control-Expose-Headers`, so a browser frontend on another origin can read it with `fetch`.

## From a script

The editor uses the control API ([control-api.md](control-api.md#payload-edits)): `PUT /__kaanal/payload` with `{ "route", "status", "body", "headers" }`, `DELETE /__kaanal/payload?route=...`, `GET /__kaanal/payload`.
