---
title: "Edges, failure and retries"
sidebar:
  order: 4
---

## Edges: who may call whom

An edge `{ from: a, to: b, net: 1ms, calls: 2 }` means `a` depends on `b`. `net` is the network latency (default zero). `calls` is how many times `b` is called per request (default 1, at least 1; active now: how many times a chain calls the edge, at most 1 000 when repeated). `prob` (above 0, at most 1, default 1) is the share of requests that take the edge; see [Endpoints](/docs/guides/mock/endpoints/#steps). `bandwidth` (bytes per second, at least 1) limits the link: with a payload size a transfer takes `bytes / bandwidth` on a FIFO link, and without it the link costs nothing and adds no hop. No cycles, no self edges, no duplicate edges.

| From                                           | May call                                                                                       |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `gateway`                                      | `service`, `monolith`, `auth`, `load-balancer`                                                 |
| `service`, `monolith`                          | `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer`, `storage` |
| `auth`                                         | `service`, `monolith`, `database`, `cache`, `external-api`, `load-balancer`, `storage`         |
| `load-balancer`                                | `service`, `monolith`, `database`, `external-api`, `auth`, `load-balancer`, `storage`          |
| `database`, `cache`, `external-api`, `storage` | nothing                                                                                        |

An edge that breaks the table is rejected with a message listing what that kind may call.

An edge `gateway -> auth` makes every request pay an auth check; logins run inside it.

## When a call fails: retries, abandon, breaker and detect

All four are **off unless written**, so a file without them gives the same figures as before. They go on an **edge**
(a `retry` can also go on a `call:` step, which wins) and they are what turns one slow callee into a retry storm, or
stops it.

```yaml
edges:
  - from: gateway
    to: app
    retry: { max_attempts: 3, budget: {} }
    abandon: continue
    breaker: { window: 50, min_calls: 50, open_for: 10s }
  - { from: app, to: db, detect: 10s }
```

**Retries: `retry`.** A call that fails with a timeout, a 5xx (500, 502, 503, 504), a 429 or a reset is tried again after
a wait; any other failure (a 404, a 400) is never retried. `max_attempts` counts the first try (2 to 5, default 3). The
wait before the first retry is `backoff` (default 50 ms), grows by `multiplier` (default 2) up to `cap` (default 20 s)
and is randomised by `jitter` (`full` by default, so each wait is anywhere from 0 to the whole wait, as the AWS SDK and
gRPC do; `equal` or `none`). A `Retry-After` from the failure is a floor under the wait. A **write** to a database or
service is not retried unless `writes: true` (a retried write can apply twice); a write to a `storage` node always is
(S3 `PUT` and `DELETE` are idempotent). Every try is a full call: it loads the callee again, which is how retries
amplify load (for a failure rate `p` and three tries a call makes `1 + p + p^2` calls on average, and the simulator's
tests check that figure).

**Retry budget: `retry.budget`.** gRPC's retry throttling (`max_tokens` 10, `token_ratio` 0.1): the caller holds a count
of tokens, each failed call spends one, each success gives `token_ratio` back, and while half the tokens or fewer are
left no retry is made. It is kept **per caller copy**, as a real client keeps one per process. It is what stops a retry
storm from feeding itself.

**When the caller gives up: `abandon`.** A call that times out is, by default (`cancel`), dropped where it can be: work
not started does not start and queued work leaves its queue. Most servers are not told that the caller left, so the
realistic setting is `abandon: continue`: the callee finishes the call's work anyway and carries its load. With retries
on, this is the metastable failure of Bronson et al., "Metastable Failures in Distributed Systems" (HotOS 2021): a
callee at 60% load with a short timeout and `max_attempts: 3` tips into failing most requests, at a load it carries
without retries, and a `budget` or a `breaker` brings it back.

**Circuit breaker: `breaker`.** The resilience4j count-based breaker, **per caller copy and wire**. It remembers the last
`window` calls (default 100); once it has at least `min_calls` of them (default 100, never more than `window`) and the
share that failed is at or above `failure_rate` (default 0.5) it **opens**: for `open_for` (default 60 s) every call is
refused at once with a 503 and never reaches the callee. Then it lets `half_open_calls` calls through (default 10): if
fewer than `failure_rate` of them fail it closes, otherwise it opens again. A failure is a 5xx, a timeout or a reset (a
4xx does not count). `breaker: {}` takes all the defaults. With retries the order is **retry outside, breaker inside**:
each try asks the breaker, a refused try still waits its backoff and takes no budget token. Refused calls are counted
as `short_circuited` on the callee, and a run that has them says so in its warnings.

**Noticing a downed copy: `detect`.** A balancer that keeps sending to a copy that has gone down until a health check
fails is the usual way a single crash becomes a spike of errors. `detect` (default 0, at once) is how long that takes
(the health-check interval times its unhealthy threshold). It goes on a `load-balancer` node (for the copies behind it)
or on an edge to a component with copies; an edge's own value wins. A copy that goes down at instant `T` of a run (a
scenario entry with `down: true` and `from_ms`, see [scenarios](/docs/guides/break/scenarios/)) fails the calls sent to it (a refused
connection, fast) until `T + detect`, then is skipped. A copy that is down from the start of the run is already noticed.

Not modelled yet, and noted in the model audit: a slow-call trip on the breaker, the outcomes of abandoned work in the
breaker, recovery time for a downed copy, a hang instead of a refusal, passive ejection of a failing host, and `detect`
on the entry and cache picks.
