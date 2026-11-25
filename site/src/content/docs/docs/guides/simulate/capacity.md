---
title: "Limits, queues and 429s"
sidebar:
  order: 4
---

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

- **Slot duration:** held for the hop's full duration, including chaos latency.
- **Waiting on calls (`hold: call`):** by default, a node is not busy while waiting on downstream calls. Set `hold: call` to keep the worker slot held until its calls return (see [resources](/docs/reference/resources/#what-a-slot-covers-hold)); queue waits appear on the node's first own hop.

## Rate limits

`rate_limit` is a token bucket, on any node kind:

```yaml
nodes:
  nginx: { type: gateway, rate_limit: { rps: 100, burst: 20 } }
```

- Starts full with `burst` (`>= 1`) tokens, refilling at `rps` per second (`> 0` to `1,000,000,000`, with `burst / rps <= 1 year`).
- Each request takes one token on arrival; empty buckets reject with `429`.
- Token spacing rounds to whole nanoseconds — negligible up to ~10k rps, noticeable (several %) in the tens of millions.

## What counts as a visit

- **Rate limits:** take one token per **visit** (a run of consecutive hops on one node, e.g. a monolith's processing + inline DB op). Each separate call, `repeat` pass (`repeat: 3` = 3 tokens), or `parallel` branch is its own visit.
- **Slots (`concurrency`/`pool`):** taken per hop while it runs. With `hold: call`, one slot and one rate-limit token are taken on admission, covering all own hops in that call.

## What clients see

| Case                         | What the client sees                                                                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Rate limits (`429`)**      | Returns `429` with a `Retry-After` header in whole seconds (rounded up, min `1`), exposed to browser JS.                                          |
| **Rate limits on `storage`** | [`storage`](/docs/reference/nodes/#storage) nodes return `503` ("Slow Down") with no `Retry-After`, though summaries still count them as limited. |
| **Full queues**              | Return `overflow_status` (`503` by default).                                                                                                      |
| **`error_map`**              | Applies to both. If it remaps `429` to another status, `Retry-After` is dropped (only rate-limit `429`s carry it).                                |
| **Chaos precedence**         | A `down` node fails with its chaos status and consumes no token or slot.                                                                          |
| **Chain termination**        | Any limit failure ends the chain; queued `parallel` branches leave their queues, while running branches finish.                                   |

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
