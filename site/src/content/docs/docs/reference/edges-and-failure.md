---
title: "Edges, failure and retries"
sidebar:
  order: 4
---

## Edges: who may call whom

An edge `{ from: a, to: b, net: 1ms, calls: 2 }` declares that `a` depends on `b`. Cycles, self-edges, and duplicate edges are rejected on load.

| Field       | Meaning                                                                                                                                | Default and range                                   |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `net`       | Network latency                                                                                                                        | `0`                                                 |
| `calls`     | How many times `b` is called per request                                                                                               | `1`, min `1`; up to `1000` when repeated in a chain |
| `prob`      | Share of requests taking this edge; see [Endpoints](/docs/guides/mock/endpoints/#steps)                                                | `1` (`> 0` to `1`)                                  |
| `bandwidth` | Bytes per second (`>= 1`). With a payload size, transfer takes `bytes / bandwidth` on a FIFO link; without one, it adds no cost or hop | none                                                |

| From                                           | May call                                                                                       |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `gateway`                                      | `service`, `monolith`, `auth`, `load-balancer`                                                 |
| `service`, `monolith`                          | `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer`, `storage` |
| `auth`                                         | `service`, `monolith`, `database`, `cache`, `external-api`, `load-balancer`, `storage`         |
| `load-balancer`                                | `service`, `monolith`, `database`, `external-api`, `auth`, `load-balancer`, `storage`          |
| `database`, `cache`, `external-api`, `storage` | nothing                                                                                        |

An edge that breaks the table is rejected with a message listing what that kind may call. An edge `gateway -> auth` makes every request pay an auth check; logins run inside it.

## When a call fails: retries, abandon, breaker and detect

All four mechanisms are **off unless configured**, so files without them behave unchanged. They live on an **edge** (though a `call:` step's `retry` overrides the edge) and model retry storms and circuit breaking.

```yaml
edges:
  - from: gateway
    to: app
    retry: { max_attempts: 3, budget: {} }
    abandon: continue
    breaker: { window: 50, min_calls: 50, open_for: 10s }
  - { from: app, to: db, detect: 10s }
```

### Retries (`retry`)

- **What retries:** timeouts, `5xx` (`500`, `502`, `503`, `504`), `429`, and `reset`. Other failures (`404`, `400`) never retry.
- **Writes:** database/service writes do not retry unless `writes: true`. Writes to `storage` always retry (S3 `PUT`/`DELETE` are idempotent).
- **Load amplification:** every retry is a full call that loads the callee again (with failure rate `p` and 3 tries, average calls = `1 + p + p^2`).

Attempts and backoff are set with these fields:

| Field          | Meaning                                              | Default |
| -------------- | ---------------------------------------------------- | ------- |
| `max_attempts` | Attempts including the first try (`2`–`5`)           | `3`     |
| `backoff`      | Initial wait                                         | `50ms`  |
| `multiplier`   | Wait scales by this each retry                       | `2`     |
| `cap`          | Upper bound for the wait                             | `20s`   |
| `jitter`       | Randomisation of the wait: `full`, `equal` or `none` | `full`  |

A `Retry-After` header acts as a floor.

### Retry budget (`retry.budget`)

Uses gRPC-style throttling, tracked **per caller copy**:

| Field         | Default |
| ------------- | ------- |
| `max_tokens`  | `10`    |
| `token_ratio` | `0.1`   |

- Each failed call spends 1 token; each success returns `token_ratio` tokens.
- While half the tokens or fewer remain, retries are suppressed — stopping a retry storm from feeding itself.

### Abandon (`abandon`)

- **`cancel` (default):** on timeout, work not yet started is skipped and queued work leaves its queue.
- **`continue`:** the callee finishes the work anyway and carries its load (realistic for most servers). Combined with `max_attempts: 3`, this reproduces metastable failures where 60% load tips into widespread failure until a `budget` or `breaker` intervenes.

### Circuit breaker (`breaker`)

Uses resilience4j's count-based breaker, tracked **per caller copy and wire** (`breaker: {}` uses all defaults):

| Field             | Meaning                                                | Default |
| ----------------- | ------------------------------------------------------ | ------- |
| `window`          | Number of most recent calls tracked                    | `100`   |
| `min_calls`       | Calls needed before the breaker can open (`<= window`) | `100`   |
| `failure_rate`    | Failure rate that opens it                             | `0.5`   |
| `open_for`        | How long it stays open                                 | `60s`   |
| `half_open_calls` | Calls let through after `open_for`                     | `10`    |

- **Opening:** once the window has `min_calls` and the failure rate reaches `failure_rate`, the breaker **opens** for `open_for`, immediately returning `503` without hitting the callee.
- **Half-open:** after `open_for`, it lets `half_open_calls` through. If fewer than `failure_rate` fail, it closes; otherwise it re-opens.
- **Counted failures:** `5xx`, timeouts, and `reset` (`4xx` does not trip the breaker).
- **Interaction with retries:** **retry outside, breaker inside** — each try checks the breaker, and a refused try still waits its backoff without spending a budget token. Refused calls count as `short_circuited` and appear in run warnings.

### Health detection (`detect`)

- Sets how long it takes a `load-balancer` (or an edge to a multi-copy component; edge wins) to notice a crashed copy (default `0`, instant).
- When a copy goes down at `T` (e.g., `down: true` with `from_ms` in [scenarios](/docs/guides/break/scenarios/)), calls routed to it fail fast with connection refused until `T + detect`, then stop routing there.
- Copies that are already down at run start are skipped immediately.

### Not modelled yet

Slow-call breaker trips, abandoned work outcomes in the breaker, downed-copy recovery time, hangs instead of connection refusals, passive host ejection, and `detect` on entry/cache picks.
