# Request ids, logs and traces

A test failed and the screenshot says "Something went wrong". Which request was it, and what did the mock do to it? Every response from a mock route carries a short **request id**. Kaanal keeps the last 1000 requests in memory, with the hops of each, and two commands show them: `kaanal logs` for the list and `kaanal trace` for one request as a waterfall.

## The request id

```bash
curl -i http://localhost:8080/users/42
```

```http
HTTP/1.1 200 OK
x-kaanal-trace: nginx.proc=2.5ms,net=1.1ms,app.proc=4.8ms,net=0.9ms,db.read=3.7ms
x-request-id: rcb02-0002
access-control-expose-headers: x-kaanal-trace, x-kaanal-scope, x-request-id, retry-after
```

An id looks like `rcb02-0002`: the letter `r`, four hex digits, a dash and a counter.

- The four hex digits are drawn once each time `kaanal up` starts, so an id from an earlier run is never mistaken for a request of this one.
- The counter starts at 1 and grows by one per request, in the order requests **arrive**. It has at least four digits (`0042`), and grows past them (`12345`). Hex digits are lowercase.
- `kaanal reset` empties the log but the counter keeps going, so an id is never reused within a run.

**Which responses carry it.** Every response from a mock route: `200`, a chaos `503`, a pinned answer, a `reset` or `malformed` wire failure, a `404`, a `405`, a `400` for a bad `X-Kaanal-Scenario`, a `429`, and the `500` of a stopped engine. CORS preflights (`OPTIONS`) and everything under `/__kaanal/` get **no** id and take no number. The header is listed in `Access-Control-Expose-Headers`, so JavaScript on another origin can read it from a `fetch` response.

**Your own id.** If the request has an `X-Request-Id` header that matches `[A-Za-z0-9._-]` and is 1 to 64 bytes long, Kaanal records it as the entry's `client_id`. Anything else (a space, a control byte, 65 bytes, bytes that are not text) is ignored without an error. The response still carries **Kaanal's** id, never yours, so the two cannot be confused. `kaanal trace` accepts either.

## What the log holds

The log is a ring of the **last 1000 finished requests**, in memory. Restarting `kaanal up` empties it, and so does `kaanal reset` (the id counter is the only thing that carries on). The ring is ordered by when requests _finish_: a slow request that started first shows up after the fast ones that overtook it, and is still there when it ends.

Each entry holds:

| Field                    | Meaning                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`, `client_id`, `seq` | Our id, your `X-Request-Id` (or `null`), and the id's number.                                                                                                                                                                                                                                                                                                |
| `done`                   | Completion order (1, 2, 3 ... in the order requests finished). `logs?after=N` and `logs -f` use it.                                                                                                                                                                                                                                                          |
| `ts_ms`                  | When it finished, UTC milliseconds since the epoch.                                                                                                                                                                                                                                                                                                          |
| `method`, `path`         | Method and path, no query string. A path over 2 KiB or a method over 16 bytes is cut and ends in `…`.                                                                                                                                                                                                                                                        |
| `endpoint`               | The matched route template, such as `GET /users/{id}`; `null` for a `404` or `405`.                                                                                                                                                                                                                                                                          |
| `status`                 | What was sent. `499` means the client left first (see below).                                                                                                                                                                                                                                                                                                |
| `ms`                     | **Measured** wall time, from the moment the request got its id until the answer was ready.                                                                                                                                                                                                                                                                   |
| `trace_ms`               | **Simulated** total: the end of the last hop.                                                                                                                                                                                                                                                                                                                |
| `scope`                  | The `X-Kaanal-Scenario` stack that answered it; `""` for an empty header; `null` without one (and for a bad header).                                                                                                                                                                                                                                         |
| `pin`                    | The UI state a [pin](pins.md) forced, or `null`.                                                                                                                                                                                                                                                                                                             |
| `flags`                  | Why it looked the way it did: the table below.                                                                                                                                                                                                                                                                                                               |
| `failed_node`, `kind`    | The node whose failure the client saw, and `reset`, `malformed` or `timeout` when it failed that way. Both are `null` under a pin, which hides the failure from the client.                                                                                                                                                                                  |
| `chain_error`            | What the call chain failed with, even when a pin hid it or an `error_map` changed the status: `{ node, status, kind, cause }`. `kind` is `status`, `reset`, `malformed` or `timeout`; `cause` is `chaos`, `rate-limit`, `overflow` or `deadline`.                                                                                                            |
| `rejected`               | Set when no call chain answered: `unmatched`, `bad-scope`, `engine-stopped` or `client-gone`.                                                                                                                                                                                                                                                                |
| `trace`, `hops`          | The `X-Kaanal-Trace` text and the same hops as data: `node`, `kind`, `op`, `start_ms`, `dur_ms`, `wait_ms`, `lane`, `fork`, `cut`, `why`, `attempt` (2 or more on a hop inside a retry, absent otherwise). In the trace text a retry starts with a marker `retry=2/3` (attempt 2 of 3). At most 512 hops (`truncated` says more existed) and 16 KiB of text. |

**Never stored:** request or response bodies, request headers (an `Authorization` header or a cookie never reaches the log), and query strings. The one header read is `X-Request-Id`, after the check above. The **path** is stored, so do not put a secret in a path.

### Why a request looks the way it does: flags

Each flag names a cause, so a failing test does not need guessing.

| Flag | Name             | Set when                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ---- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `FI` | Fault injection  | Chaos failed the request (an `error_rate` roll, `down`), or injected a hang. A hanging **cache lookup** does not fail the request: it answers `200`, the hop's note says `chaos hang`, and the flag is `FI`.                                                                                                                                                                                                                   |
| `DI` | Delay injection  | Chaos `latency_ms` was actually added to a hop that ran (the hop's note says `chaos +40ms`), or a pin delay. A hang is never `DI`: it is a failure, so it is `FI` (or `DT`, below). A link hop can carry it too: `chaos +20ms`, `link jitter ±5ms` or `link capped at N B/s`; with several edges into one node the net hop is not blamed on a link (it does not say which edge it crossed). A link reset or partition is `FI`. |
| `PD` | Pin delay        | A `loading` or `slow` [pin](pins.md) delayed the request. A pin delay always shows as `PD`, `DI` and `PN` together.                                                                                                                                                                                                                                                                                                            |
| `RL` | Rate limited     | A node's `rate_limit` ran out of tokens (`429`). See [capacity](capacity.md).                                                                                                                                                                                                                                                                                                                                                  |
| `UO` | Queue overflow   | A full queue refused the request (the node's `overflow_status`, `503` by default).                                                                                                                                                                                                                                                                                                                                             |
| `DT` | Deadline timeout | A `timeout:` fired. A hang on a node that has a `timeout` ends this way, so it is `DT`; a hang on a node with no `timeout` runs to the 60 s cap and is `FI`.                                                                                                                                                                                                                                                                   |
| `OM` | Out of memory    | A node with `memory: { on_limit: kill }` was killed with the request in it (`502`), or was down from a kill and refused it (`503`). See [nodes](nodes.md#killing-the-copy-on_limit-kill).                                                                                                                                                                                                                                      |
| `PN` | Pinned           | A [pin](pins.md) decided the answer.                                                                                                                                                                                                                                                                                                                                                                                           |

When the chain failed and a pin hid it, `chain_error` and the flag of the real cause (`FI` here) are still there next to `PN`: the trace shows `pinned=success hid: db.error=503`.

## `kaanal logs`

```bash
kaanal logs                       # the last 50 requests
kaanal logs -n 200                # the last 200 (1 to 1000)
kaanal logs --status 5xx          # 404, 404,5xx, 5xx ...
kaanal logs --failed              # only requests a node failed
kaanal logs --route "GET /users/{id}"
kaanal logs --since 5m            # a duration, or an RFC 3339 time
kaanal logs --scope payments-down # requests that ran under that scenario scope
kaanal logs --scope ""            # requests sent with an empty X-Kaanal-Scenario header
kaanal logs --json                # one JSON object per line (the full entry); --json is global
kaanal logs -f                    # follow live
```

One line per request, oldest first:

```
17:31:34.719Z  rcb02-0001  GET /users/42  200  31ms
17:31:34.907Z  rcb02-0003  GET /users/7  200  21ms  client=checkout-17
17:31:34.985Z  rcb02-0004  GET /nope  404  0ms  unmatched
17:31:35.893Z  rcb02-0005  GET /users/1  200  64ms
17:31:36.069Z  rcb02-0006  GET /users/9  503  22ms  failed=db
17:31:36.644Z  rcb02-0007  GET /users  500  39ms  pinned=error
```

The columns are the UTC time, the id, the request, the status and the measured time. After them come `scope=`, `pinned=`, `failed=`, `kind=` and `client=` when they apply, and the `rejected` word (`unmatched`, `bad-scope`, `engine-stopped`, `client-gone`).

Filters combine (all must match). Things to know:

- `--route` (formerly `--endpoint`, still accepted for one release) takes the **route template as logged**, `GET /users/{id}`, not a concrete URL. The method's case and the spacing are normalised (`get /users/{id}` works). `GET /users/1` is a valid text that names no route, so it matches nothing, silently: use the template. A `404` has no endpoint, so find those with `--status 404`.
- `--status` takes exact codes and classes, comma separated: `404`, `5xx`, `404,5xx`.
- `--failed` keeps requests with a `failed_node`. A pinned request never counts, because the client did not see the failure.
- `--since 5m` keeps requests that finished in the last five minutes; an RFC 3339 time such as `2026-10-18T09:00:00Z` works too.
- `-n` is 1 to 1000 (default 50). The tail is the **latest** n that match.
- A closed pipe (`kaanal logs | head`) ends quietly with exit code 0.

### `-f`: follow

`kaanal logs -f` prints the tail first, then every request as it finishes, until you press Ctrl+C (exit code 0). Filters apply to the live lines too, with the same rules. It is built so that nothing is lost or doubled:

- It **subscribes first**, then fetches the tail, then prints the tail and whatever arrived meanwhile. A request that finishes between the two is in both and printed once (the follower remembers the last 2000 ids).
- Lines come in the order the events are published, which is the order requests **finish** give or take a few that finish at nearly the same moment (the log and the event stream are two steps). A slow request started early is printed when it ends, not before the fast ones that overtook it. `done` and `logs?after=` are exact.
- If the screen cannot keep up (the follower buffers 4096 events), it says so instead of falling silent: `-- missed 17 requests (follow slower than the server) --`. The server sends the same notice when it drops events itself. Use `kaanal logs --since` to read what was missed.
- The event stream is opened with an `Authorization` header, so the token never appears in a URL.
- `--json` is the [global flag](user-guide.md#global-flags) (`kaanal --json logs` works too, and so does `KAANAL_JSON=1`); the output is the same as before. With it, stdout carries JSON Lines only: each live line is the full entry, fetched when the event arrives. If the entry left the ring in between, the line is the shorter event object instead (it has the same `id`, `status`, `path`, `endpoint` and so on, but no `hops`), and `-- could not fetch ID --` goes to stderr. The `-- missed N requests ... --` notice goes to stderr too, so a pipe into `jq` stays clean.
- If the server stops, `logs -f` ends, because the event stream does, and it exits with code 2. A `kaanal down` (or a signal) makes the server send a `shutdown` event first; `logs -f` then waits for the process to finish its exit and prints `kaanal stopped (the server shut down)`. A server that was killed has no such event: the follower waits up to 2 s for the lock, then reports `kaanal is not running on ...: the event stream closed`. Only a server that is still alive (the lock stays held) gives exit 1, `kaanal is running (pid N) but its control API did not answer`. See [lifecycle](lifecycle.md#stale-state). The events of requests cut at the end of a shutdown do not reach a follower.

## `kaanal trace`

```bash
kaanal trace last                # the latest request to finish
kaanal trace rcb02-0005          # by Kaanal's id
kaanal trace checkout-17         # by the X-Request-Id you sent (the newest match)
kaanal trace rcb02-0005 --json   # the whole entry as JSON (--json is the global flag)
```

A request that took 40 ms of chaos on the database:

```
rcb02-0005  GET /users/1  200  served 64ms  trace 49.5ms  flags=DI
  nginx.proc       0.0ms       0.8ms  |#                                       |
  app.net          0.8ms       1.2ms  |#                                       |
  app.proc         2.0ms       2.0ms  | ##                                     |
  db.net           4.0ms       0.9ms  |   #                                    |
  db.read          4.9ms      44.6ms  |   #####################################|  chaos +40ms
```

The header gives the id, the request, the status, the measured time (`served`), the simulated total (`trace`), and the scope, pin, client id and flags when present. Each row is one hop: its node and kind, when it started and how long it ran (simulated ms), then a 40-column bar of the total, and a note.

- `#` is the hop running. A hop always gets at least one column, so a 0.1 ms hop is still visible.
- `.` before it is time spent **queued** for a slot at that node ([capacity](capacity.md)); for a node with `hold: call` it is the wait for the slot the node holds across its calls, shown on its first own hop.
- `x` is a hop that was **cut**: it ran for less time than planned because a failure or a deadline stopped it, or it is the hop that failed.
- The note shows why: `chaos +40ms`, `chaos hang`, `pin delay`, `queued 98.0ms`, and `lane 1`, `lane 2` for the branches of a `parallel` block. (A nested `parallel` shows only its innermost branch number.)
- The last line says how it ended: `failed at db: 503 (chaos)`, `failed at db: 504 timeout (deadline)`, `failed at gw: 429 (rate-limit)`, or, for a pin that hid a failure, `pinned=success hid: db.error=503`.

A request the chain never answered has no hops:

```
rcb02-0004  GET /nope  404  served 0ms  trace 0.0ms
  - (unmatched)
```

A pin that hid a failure:

```
rcb02-0010  GET /users/5  200  served 15ms  trace 8.3ms  pinned=success  flags=FI,PN
  nginx.proc       0.0ms       0.4ms  |##                                      |
  app.net          0.4ms       1.2ms  |  #####                                 |
  app.proc         1.6ms       5.2ms  |       ##########################       |
  db.net           6.9ms       1.4ms  |                                 xxxxxxx|
  pinned=success hid: db.error=503
```

And a chaos failure the client saw:

```
rcb02-0009  GET /users/3  503  served 14ms  trace 6.0ms  flags=FI
  nginx.proc       0.0ms       0.9ms  |#####                                   |
  app.net          0.9ms       0.6ms  |     #####                              |
  app.proc         1.5ms       3.2ms  |          #####################         |
  db.net           4.7ms       1.3ms  |                               xxxxxxxxx|
  failed at db: 503 (chaos)
```

### When an id is not found

| You asked for                                                               | Answer                                                                             |
| --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| An id of this run that left the ring, or is still in flight                 | `HTTP 404: evicted (older than the last 1000 requests, or still in flight)`        |
| An id from another run, a number not issued yet, or a client id nobody sent | `HTTP 404: unknown`                                                                |
| Text that is neither an id, `last` nor a valid client id                    | `HTTP 400: expected a request id like r04f2-0042, 'last' or a client X-Request-Id` |
| `last` with nothing recorded yet                                            | `no requests recorded yet`                                                         |

All of them exit with code 1. A server that is not running exits with code 2. An id that is **in flight** reads `evicted` only if a later request has already finished; wait for the answer, then ask again.

### Measured against simulated time

`served` (`ms`) is the time a client really waited. `trace` and every row are the **simulated** times the plan was built from. They differ: the first includes scheduling, the second does not. On Windows the OS timer ticks about every 15 ms, so a short request can be _served_ 15 ms later than its trace says, while the hops themselves start at their scheduled times (`served 96ms`, `trace 79.3ms` is typical for an 80 ms chain). Read the waterfall for how the chain behaved, and `served` for what your test experienced.

## Using it from a test runner

Read the id from the response that failed, then ask for its trace when the test fails.

Playwright:

```ts
test('checkout shows the error banner', async ({ page }) => {
  const failed = page.waitForResponse((r) => r.url().includes('/orders') && !r.ok());
  await page.getByRole('button', { name: 'Pay' }).click();
  const res = await failed;
  const id = res.headers()['x-request-id'];     // e.g. rcb02-0042
  console.log(`kaanal trace ${id}`);            // paste it into a terminal
  await expect(page.getByText('Payment failed')).toBeVisible();
});
```

Cypress:

```js
cy.intercept('POST', '/orders', (req) => {
  req.continue((res) => {
    Cypress.log({ name: 'kaanal', message: `kaanal trace ${res.headers['x-request-id']}` });
  });
});
```

Or stamp your own id on a request and look it up without reading any header:

```ts
await page.setExtraHTTPHeaders({ 'X-Request-Id': `pay-${testInfo.testId}` });
// later, in a terminal:  kaanal trace pay-<testId>
```

A browser on another origin can read `X-Request-Id` because Kaanal exposes it through CORS. To list everything a test did, give each test a [scenario scope](scenarios.md#parallel-test-workers) and run `kaanal logs --scope NAME`, or call [`GET /__kaanal/logs`](control-api.md#get-__kaanallogs) yourself.

## Requests that did not finish normally

- **`client-gone`.** The client closed the connection before the answer was ready (a test timed out, a page navigated away). The entry has status `499` (nginx's "client closed request"), `rejected: client-gone`, no hops, and the endpoint and scope the request had by then, and the SSE stream gets its event. The simulated request still runs to its end inside Kaanal (it holds the queue slots a real request would), but nothing more is logged for it.
- **`engine-stopped`.** The engine behind the mock is gone: `500 {"error":"engine stopped"}`. It is logged, with the endpoint and the scope, and the route's request counter still advanced and its plan was still drawn (that is how the counter has always worked; it only differs from a `404` or `400`).
- **A handler that panics** logs nothing. That would be a bug in Kaanal; the id it took is simply never seen again.
- **`unmatched` and `bad-scope`.** A `404`, a `405` and a `400` for a bad `X-Kaanal-Scenario` are logged with those words, and take no counter value and no random draw, so a seeded run is not disturbed by stray requests.

## Limits

- 1000 entries, 512 hops per entry, 16 KiB of trace text per entry, 2 KiB of path.
- `GET /logs?limit=1000` clones the ring while it holds the lock; with 512-hop entries that can be on the order of 100 MB in the worst case, and requests finishing at that moment wait for the clone. Ask for fewer entries (`-n`, `limit`) or filter, when the entries are big.
- `logs -f` drops a single event line or message larger than 1 MiB and counts it as missed.
- The log is in memory and per process: not shared between two `kaanal up` instances, and gone on restart.
- A request id is an index into a window, not an audit trail: under heavy load, ids of the first requests can fall out in seconds. Take the trace while you are looking at the failure.
