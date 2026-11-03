# Limits, queues and 429s

A real server runs out of room: a database has a few connections, an API allows so many calls per second. Kaanal models both, so your frontend meets the same slow spinners, "server busy" pages and `429`s it will meet in production. Live mode and the simulator use the same rules.

## Slots and queues

`concurrency` is how many requests a node serves at once. A request that finds every slot busy **waits in line** and starts when a slot frees (first in, first out).

```yaml
nodes:
  app: { type: service, spec: openapi.yaml, concurrency: 4 } # 4 at once, the rest wait, no limit on the line
```

Add a bounded line and an overflow status with the long form:

```yaml
nodes:
  app: { type: service, spec: openapi.yaml, concurrency: { limit: 4, queue: 10, overflow_status: 503 } }
```

With `queue: 10`, the 11th waiting request is rejected at once with `overflow_status` (500 to 504, default 503). The queue defaults to 0 in the long form: no waiting, a full node rejects.

`pool` is the same thing for `database` and `external-api` nodes, under the name people expect:

```yaml
nodes:
  db: { type: database, flavor: postgres, pool: { size: 2, queue: 3 } }
```

Set one of `pool` or `concurrency` on a node, not both (the file will not load).

A slot is held for a hop's whole duration, including any extra latency you add with chaos. By default that is all: a node that waits on what it calls is not busy while it waits. A node that keeps a worker per request can say so with `hold: call`, which holds the slot until its calls return (see [nodes](nodes.md#what-a-slot-covers-hold)). A request that waits for a held slot shows the wait on the node's first own hop.

## Rate limits

`rate_limit` is a token bucket, on any node kind:

```yaml
nodes:
  nginx: { type: gateway, rate_limit: { rps: 100, burst: 20 } }
```

The bucket starts full with `burst` tokens and refills at `rps` per second. Each request takes one token when it reaches the node; with none left it is rejected with `429`. `rps` must be above 0 (at most 1000000000), `burst` at least 1, and `burst / rps` can be at most a year. Internally the time between tokens is rounded to a whole nanosecond, so the real rate is off by at most half a nanosecond per token: negligible up to about 10 000 requests per second, and noticeable (several percent or more) in the tens of millions.

## What counts as a visit

A rate limit takes one token per **visit**, not per hop. A visit is a run of consecutive hops of one node, so a monolith's own processing followed by its inline database operation is one visit. A second call to the same node is a visit of its own, each pass of a `repeat` is one (`repeat: 3` takes three tokens), and a `parallel` block takes one per branch. Slots work per hop: each hop of the node takes a slot while it runs. With `hold: call` the node takes one slot (and one rate-limit token) when the request is admitted, and its own hops in that call run inside it.

## What clients see

- **A rate limit says `429`** with a `Retry-After` header in whole seconds (rounded up, at least 1). A frontend can read it from JavaScript, because Kaanal exposes the header to browsers.
- **A full line says the overflow status** (503 unless you changed it).
- **`error_map` applies to both**, like any other failure. If it turns a 429 into something else, the `Retry-After` header is dropped: it only appears while the final status is 429 and a rate limit caused it (a 429 you create with `error_map` from another failure carries no `Retry-After`).
- **Chaos wins over a limit.** A node you have set `down` fails with its own status and uses no token and no slot, so it never becomes a 429.
- **A failing request ends the whole chain**, as with any failure. Branches of a `parallel` block that were still waiting in a line leave it; branches already running finish.

## Seeing the wait

The `X-Kaanal-Trace` header gains `node.wait=Tms` before a hop that had to wait:

```
x-kaanal-trace: nginx.proc=1.0ms,net=1.0ms,app.proc=2.0ms,net=1.0ms,db.wait=98.0ms,db.read=100.0ms
```

At zero load there is no `wait` piece, so traces look exactly as before. A failure from a limit ends the trace with the usual `node.error=429` (or the overflow status).

On Windows the answer can arrive up to about one timer tick (15 ms) after the trace says it should. The hops inside the trace are still exact.

## In the simulator

Kaanal's offline simulator runs the same rules on a virtual clock and reports per node the waits, the queue depth and utilisation, and two new counts: `limited` (requests turned away by a rate limit) and `overflowed` (requests turned away by a full line). At zero load its latencies equal live mode's exactly.

## Not modelled

- **Changing a limit while Kaanal runs.** Limits come from `kaanal.yaml`; edit the file and restart. (Runtime control comes with the control API.)
- **Limits on edges.** Limits belong to nodes.
