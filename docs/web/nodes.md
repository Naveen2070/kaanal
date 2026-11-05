# Nodes reference

Every node kind you can put under `nodes:`, its fields, and what each field does today. Set the kind with `type:`. A field that does not belong to a kind is rejected when the file loads.

Fields marked **active now** change the simulation. Fields marked **not active yet** are accepted and checked (a bad value is still an error) so your file will not need rewriting later, but they have no effect on timing yet.

How a request walks these nodes, and how to change the walk per route, is in the [endpoints reference](endpoints.md).

Durations are `5ms`, `{ p50: 4ms, p99: 20ms }` or `{ mean: 5ms }`, as described in the [user guide](user-guide.md).

## `gateway`

A reverse proxy or API gateway (nginx, Envoy). Optional, at most one. For a balancer that spreads calls over several components, see [`load-balancer`](#load-balancer). With a gateway, every request pays its `proc` first, then runs its auth calls, and it answers static endpoints (an `endpoints:` entry with `override` and no owning service).

```yaml
nginx: { type: gateway, flavor: nginx, proc: 1ms }
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `flavor`      | Free-text label shown in status output (`nginx`, `envoy`)                                                                                                                                                                                      | none      | active now |
| `proc`        | Processing time per request                                                                                                                                                                                                                    | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now |

## `service`

A backend service. It may carry an OpenAPI 3.0 `spec`. Without a gateway, the routes in its `spec` are served here.

```yaml
app: { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                                                                                                                  | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                                                                                                                         | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now |

## `monolith`

A service that can also be its own database. With `ops` or a `flavor` it has inline read/write/scan costs; with edges to databases it behaves as a service. Both together are allowed.

```yaml
core: { type: monolith, spec: openapi.yaml, proc: 3ms, flavor: mysql, ops: { read: 1ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                                                                                                                  | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                                                                                                                         | zero      | active now |
| `ops`         | Inline `read`, `write`, `scan` costs; any you leave out comes from `flavor`                                                                                                                                                                    | none      | active now |
| `flavor`      | Preset that fills `ops` you left out: `postgres`, `mysql`, `mongodb`                                                                                                                                                                           | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now |

## `database`

A data store with costs for `read`, `write` and `scan`. Give all three, or a `flavor` to fill the gaps.

```yaml
db: { type: database, flavor: postgres, ops: { write: 9ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `ops`         | `read`, `write`, `scan` costs; required in full unless `flavor` fills them                                                                                                                                                                     | none      | active now |
| `flavor`      | Preset for ops you left out: `postgres`, `mysql`, `mongodb`                                                                                                                                                                                    | none      | active now |
| `pool`        | Connection pool: `size` (at least 1), `queue` (default 0), `overflow_status` (500 to 504, default 503)                                                                                                                                         | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now |

## `cache`

A cache in front of slower nodes, with a hit ratio and the cost of a hit and of a miss. Without a `flavor`, `hit_ratio`, `hit` and `miss` are all required.

```yaml
cache: { type: cache, flavor: redis, hit_ratio: 0.8 }
```

| Field         | Meaning                                                                                                      | Default     | Status                               |
| ------------- | ------------------------------------------------------------------------------------------------------------ | ----------- | ------------------------------------ |
| `flavor`      | Preset for `hit_ratio`, `hit` and `miss`: `redis`, `memcached`                                               | none        | active now                           |
| `hit_ratio`   | Share of lookups that hit, 0 to 1                                                                            | from flavor | active now                           |
| `hit`         | Cost of a hit                                                                                                | from flavor | active now                           |
| `miss`        | Cost of finding out it is a miss                                                                             | from flavor | active now                           |
| `ttl`         | Entry lifetime                                                                                               | none        | accepted and checked, not active yet |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                              | unlimited   | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429 | none        | active now                           |

A lookup slower than the cache's `timeout` counts as a miss. If `timeout` is below the `hit` cost (its median, for a distribution), most hits are cut, and loading prints a warning naming the cache.
| `timeout` | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none | active now |

## `external-api`

A third-party HTTP API such as payments or email.

```yaml
pay: { type: external-api, proc: 50ms, error_rate: 0.01 }
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ------------------------------------ |
| `proc`        | Response time of the API                                                                                                                                                                                                                       | zero      | active now                           |
| `error_rate`  | Baseline share of failing calls, 0 to 1                                                                                                                                                                                                        | none      | accepted and checked, not active yet |
| `proxy`       | Real upstream URL (`http://` or `https://`)                                                                                                                                                                                                    | none      | accepted and checked, not active yet |
| `pool`        | Connection pool, as on `database`                                                                                                                                                                                                              | none      | active now                           |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now                           |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now                           |

## `auth`

An identity provider: serves `POST /login` and `GET /.well-known/jwks.json` (see [auth.md](auth.md)). At most one.

```yaml
login:
  type: auth
  proc: 5ms
  token_ttl: 2h
  claims: { aud: my-app }
  users: [{ username: alice, password: secret, roles: [admin], scopes: [read] }]
```

| Field         | Meaning                                                                                                                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `proc`        | Cost of one login, one JWKS request or one gateway token check                                                                                                                                                                                 | zero      | active now |
| `users`       | Accounts that can log in (`username` unique, `password`, `roles`, `scopes`, `claims`)                                                                                                                                                          | none      | active now |
| `token_ttl`   | Lifetime of issued tokens, whole seconds, 1s to 1 day                                                                                                                                                                                          | 1h        | active now |
| `claims`      | Extra claims in every token (a user's own `claims` win)                                                                                                                                                                                        | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](concepts.md#chaos) | none      | active now |

## `load-balancer`

A hop that sends each call to **one** of the components it has wires to, chosen by `spread`. It has its own time, slots, queue, rate limit, timeout, copies and `bandwidth` like any proxy, no chain of its own, and it is never an entry node (only a gateway, or the service that owns a route, is). There may be several, for example one in front of each tier.

```yaml
lb:
  type: load-balancer
  proc: 200us
  concurrency: 1000
  spread: { weighted: [3, 1] }   # the wires out of lb, in the order of edges:
```

| Field                                                                | Meaning                                                                                                                                                                 | Default     |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `proc`                                                               | Cost of passing one call through                                                                                                                                        | zero        |
| `spread`                                                             | How it picks the component a call goes to: `round-robin`, `random` or `{ weighted: [..] }` with one weight per wire out of it. A component with no live copy is skipped | round-robin |
| `balance`                                                            | How traffic entering the balancer picks one of its own copies (as on every node)                                                                                        | round-robin |
| `concurrency`                                                        | A number, or `{ limit, queue, overflow_status }`                                                                                                                        | unlimited   |
| `hold`                                                               | `own` (a slot only for its own work) or `call` (until the chosen component has answered: a connection-limited proxy)                                                    | `own`       |
| `bandwidth`, `per_kb`, `rate_limit`, `timeout`, `replicas`, `locale` | As on the other nodes                                                                                                                                                   |             |

The chosen component then picks a copy by its own `balance`, and the wire to it keeps its `net`, `calls` and `bandwidth`. An edge out of a balancer takes no `prob` (the spread decides). With every callee down the call fails with a 503 at the balancer. Round robin counts the calls each copy of the balancer gets, so copies of a balancer do not lock to the components behind them. **A route behind a balancer.** When the gateway has no wire to the service that owns a route but has one to a balancer that reaches it (through one balancer or several in a row), the request crosses each balancer on the way: path-based routing, as an nginx or an ALB rule does, so the route decides the target and the balancer does not pick; its time, slots, queue, bandwidth and `hold` are paid on every such route. A direct wire from the gateway to the owner wins when there is one. To spread one route over several identical backends, give the owner `replicas` and a `balance` (a pool); `spread` is for a balancer that really chooses between different components (a call from a service, a `call:` step, a gateway chain). **Not built yet:** `least-conn` and `p2c` (they need the live queues; the next step).

## `storage`

An object store (S3, Google Cloud Storage, Azure Blob): a **leaf** that calls nothing. Whatever may call a database may call it (`service`, `monolith`, `auth`, and a `load-balancer` in front of it).

```yaml
assets:
  type: storage
  flavor: s3
  first_byte: 25ms
  throughput: 80MB/s
```

| Field                                                   | Meaning                                                                                                                                                                                                                                                     | Default            |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `first_byte`                                            | Time to the first byte of a call, the node's service time. A duration like `proc`: `25ms`, `{ p50: 20ms, p99: 80ms }` or `{ mean: 25ms }`. **Optional**: absent means 25 ms, a labelled estimate (no provider publishes one), so write your own measurement | none               |
| `throughput`                                            | Bytes per second the store moves for one call: a whole number or text like `80MB/s`, `250KB/s` (KB = 1024 bytes, see [units](#payload-cost)); at least 1 byte per second. A larger body takes longer                                                        | none: no size term |
| `flavor`                                                | `s3`, `gcs` or `blob`. A **label only** (shown on the page and in status output); it sets no number                                                                                                                                                         | none               |
| `concurrency`                                           | A number, or `{ limit, queue, overflow_status }`: calls served at once                                                                                                                                                                                      | unlimited          |
| `rate_limit`                                            | Token bucket: `rps` and `burst`; over it a call fails with **503** (S3's "Slow Down"), not 429 like the other kinds                                                                                                                                         | none               |
| `timeout`, `replicas`, `balance`, `bandwidth`, `locale` | As on the other nodes                                                                                                                                                                                                                                       |                    |

**Cost of one call** is `first_byte + (request bytes + response bytes) / throughput`, with KB = 1024. The bytes are the request and response sizes already in the traffic (a stream's `req_bytes` and `resp_bytes`; the live mock's `Content-Length` and body size), so there is no size field of its own; with no sizes, or no `throughput`, the size term is nothing. Example: `first_byte: 20ms`, `throughput: 100MB/s` and a 5 MB response cost 20 ms + 5 MiB / (100 MiB/s) = 20 ms + 50 ms = 70 ms; a 5 KB object adds under a millisecond. It runs on the same machinery as `proc` and `per_kb`: the node's service time is `first_byte` and the size term is `per_kb` worked out as `1024 / throughput` seconds per KB (held to the nanosecond, so a size term is exact to about 1 part in 10^4 at 100 MB/s). A storage node therefore refuses `proc` and `per_kb` (use `first_byte` and `throughput`), and `ops`, `pool`, `hit_ratio` and `hold` (it calls nothing, and its concurrency is `concurrency`), each as a load error that says so.

**Operations.** A step or route `op: read`, `write` or `scan` toward a storage node is accepted exactly as for a database, and means `GET`, `PUT` and `LIST`. All three cost the same `first_byte` plus the size term per request (a read, `GET`, also **streams** over a wire or card with a `bandwidth`, see below): no provider publishes a different first-byte latency for a listing, so there is no separate LIST number. What makes a listing cost more is that it **pages**: S3 returns at most 1,000 keys per `LIST` request (GCS and Azure Blob page the same way), so a listing of N keys is about N / 1,000 requests one after another, each paying `first_byte` and its own page. Write that as `- { call: assets, op: scan, repeat: 10, returns: 200KB }`: ten pages of the size one page of your keys takes (it depends on key length, so measure it). A bare `call: assets` works, and a service's default body calls its storage nodes after its database-style callees. Whether a storage call holds the caller's slot follows the caller's own `hold` setting; the node holds one of its own slots only for its own work.

**Rate limit, and why there are no presets.** `rate_limit` is the node's request rate (one limit per node; the simulator has no key prefixes). It is off unless you set it and the flavor does not set it. No provider publishes a first-byte latency for standard object storage, so **there are no presets**: `first_byte` and `throughput` are numbers you write from your own measurements. The request-rate figures that are published (searched 2026-11-04) are:

- AWS S3: at least 3,500 PUT, COPY, POST and DELETE requests and 5,500 GET and HEAD requests per second per prefix, and a sizing guideline of one request per 85 to 90 MB/s; it answers 503 Slow Down while it scales (docs.aws.amazon.com/AmazonS3/latest/userguide/optimizing-performance-design-patterns.html).
- Google Cloud Storage: about 5,000 object reads and 1,000 object writes per second per bucket to start, scaling as it goes; it exposes no latency metric (docs.cloud.google.com/storage/docs/request-rate).
- Azure Blob: up to 3,000 requests per second for one block blob; throughput is bounded by the storage account (learn.microsoft.com/azure/storage/blobs/scalability-targets).

**A storage node in Studio** starts as `{ type: storage, flavor: s3, first_byte: 25ms, throughput: 80MB/s }`. Those two numbers are **placeholders to edit, not published figures** (see above). It has no `concurrency` on purpose: the request-rate figures belong to `rate_limit`. Studio edits every field above in boxes (see the Storage entry of [Studio](studio.md)); its **Add a request limit** button writes `rate_limit: { rps: 5500, burst: 5500 }`, S3's published read rate per prefix, as a starting point to change (every other kind's button starts at `{ rps: 1000, burst: 1000 }`, a placeholder, because no provider publishes a figure for a service or a database).

## Nodes added in Studio

`add_node` (see the [control API](control-api.md#draft)) writes a node that is valid as it stands and has a capacity, so the bottleneck report can rank it. These are **estimates**: typical sizes for a small deployment, meant to be edited.

| Kind            | Written as                                                                                                                    |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `gateway`       | `{ type: gateway, concurrency: { limit: 256, queue: 100 }, timeout: 30s }`                                                    |
| `service`       | `{ type: service, concurrency: { limit: 32, queue: 100 } }`                                                                   |
| `monolith`      | `{ type: monolith, concurrency: { limit: 64, queue: 100 } }`                                                                  |
| `database`      | `{ type: database, flavor: postgres, pool: { size: 20 } }` (the flavor fills the read, write and scan costs)                  |
| `cache`         | `{ type: cache, flavor: redis }` (no capacity; the flavor fills hit ratio and costs)                                          |
| `external-api`  | `{ type: external-api, pool: { size: 10 } }`                                                                                  |
| `auth`          | `{ type: auth, concurrency: { limit: 32, queue: 100 } }` (needs a signing key, which `kaanal up` and `kaanal studio` provide) |
| `load-balancer` | `{ type: load-balancer, concurrency: { limit: 1000, queue: 100 }, proc: 200us }`                                              |
| `storage`       | `{ type: storage, flavor: s3, first_byte: 25ms, throughput: 80MB/s }` (placeholders, not published; no slots)                 |

Every node with a slot limit is written with `queue: 100`, and the gateway (the entry node) with a client `timeout: 30s`, which is the deadline for a whole request through it. Both are **estimates**: 100 waiting requests is a modest backlog, and 30 s is the low end of the 30 to 60 s where real clients and proxies give up. They are written into the file only. Unset still means unlimited (`queue`) and never (`timeout`), so a file without them behaves as before. `kaanal init` and the Studio starters write the same values.

## Edges: who may call whom

An edge `{ from: a, to: b, net: 1ms, calls: 2 }` means `a` depends on `b`. `net` is the network latency (default zero). `calls` is how many times `b` is called per request (default 1, at least 1; active now: how many times a chain calls the edge, at most 1 000 when repeated). `prob` (above 0, at most 1, default 1) is the share of requests that take the edge; see [Endpoints](endpoints.md#steps). `bandwidth` (bytes per second, at least 1) limits the link: with a payload size a transfer takes `bytes / bandwidth` on a FIFO link, and without it the link costs nothing and adds no hop. No cycles, no self edges, no duplicate edges.

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
scenario entry with `down: true` and `from_ms`, see [scenarios](scenarios.md)) fails the calls sent to it (a refused
connection, fast) until `T + detect`, then is skipped. A copy that is down from the start of the run is already noticed.

Not modelled yet, and noted in the model audit: a slow-call trip on the breaker, the outcomes of abandoned work in the
breaker, recovery time for a downed copy, a hang instead of a refusal, passive ejection of a failing host, and `detect`
on the entry and cache picks.

## Payload cost

Off by default. `per_kb` on any node (a duration like `proc`, e.g. `per_kb: 50us`) adds that much processing time for every KB of the request and of the response, once per body. `bandwidth` on an edge (a rate, see below) gives the link a rate that the transfers on it share fairly. Payload sizes come from the traffic (a stream's `req_bytes` and `resp_bytes` in the simulator; in the live mock, the request's `Content-Length` and the size of the JSON body the route would answer). A route with a collection or a built-in login or JWKS counts its response as 0 bytes. With no sizes, neither field changes anything. In a trace a transfer shows as `db.link=50.0ms` (request) and `db.link-resp=12.0ms` (response), the node being the edge's target. A cache takes `bandwidth` on its edge but no `per_kb` (a load error).

**Sizes per call: `sends` and `returns`.** By default one request size and one response size (the traffic's) are passed down every call. An edge or a `call:` step can say what that call really carries: `sends:` is the request body and `returns:` the response body, written like `req_bytes` (`200KB`, at most 1 GiB). A step beats its edge, and unset passes the incoming size on, so a file without them does not change. A 5 MB upload that the service turns into a 200 KB call to storage is `- { call: files, op: write, sends: 200KB }`; the store is charged for 200 KB, the database call after it still sees the 5 MB, and the sizes go back to the caller's when the call ends.

**Which body a node's size cost counts.** A `per_kb` cost and a storage node's `throughput` count the body that moves, as HTTP and object stores do: a **write** (`PUT`, a database insert, update or delete) counts the request body it receives; a **read** (`GET`) and a **scan** (`LIST`) count the response body it sends. Any other hop (a service doing its own work) counts both. So a download costs the object once, an upload costs it once, and a database write no longer pays for a reply it never sends. This is the rule for the `op` in force: the step's `op:`, else the endpoint's (a `GET` with an id reads, a `GET` without lists, anything else writes).

**Headers.** An HTTP call carries headers as well as a body, and they dominate small calls on slow links. `header_bytes:` at the top of the file sets them: one size for both sides (`header_bytes: 1KB`), or `{ request: 600B, response: 400B }` (a missing side keeps its default). The **default is 600 B on a request and 400 B on a response**, estimates of what an HTTP client sends (cookies and an auth token included) and a server answers, not measurements; `header_bytes: 0` turns them off. They are charged on every wire transfer (so a call with no body still crosses a wire that has a `bandwidth`) and in the `per_kb` cost of an HTTP node (a service, monolith, gateway, auth, load balancer, external API or storage node, not a database op), so a topology with no `bandwidth`, `per_kb` or `throughput` does not change at all. **Upgrading:** a file with a `bandwidth` or a `per_kb` now reports slightly longer times; set `header_bytes: 0` to get the old numbers.

**Connection set-up: `tls` and `keepalive`.** A call over an edge with a `net` rides a connection. One call in `keepalive` (default 1000, nginx's `keepalive_requests`; `1` is a new connection every call) opens a new one; the others reuse a warm connection and pay nothing extra. A new connection pays, before the callee runs, **one round trip for TCP** (RFC 9293), **one more for TLS 1.3** or **two for TLS 1.2** (`tls: 1.3`, `tls: true` or `tls: 1.2`; default none), and then TCP **slow start**: the first flight is 10 segments of 1460 B (RFC 6928) and the window doubles every round trip (RFC 5681), so a body that does not fit waits one round trip per doubling until the window fills the pipe (the narrowest of the wire and the two cards, `bandwidth x round trip`; an edge with no `bandwidth` never fills, so a 1 MiB body waits 6 round trips). A round trip is the call's own `net` draw, jitter and chaos included, so **an edge with no `net` pays no set-up**. Both directions start cold on a new connection. Not modelled: a pool that runs out under load, idle-timeout closes, slow-start after idle, session resumption and 0-RTT, HTTP/2 and HTTP/3 multiplexing, packet loss (step 6d). The set-up shows in the log as `connect` and `connect response` hops. This is on by default, so about 1 call in 1000 on an edge with a latency now takes a round trip more; `keepalive: 1000000` all but turns it off.

**Streamed responses: `stream`.** By default a response is buffered: the callee finishes all its work, then the bytes cross the wire, so a 1 MiB read from a store costs `first_byte + size / throughput` and then `size / wire`, added. A **streamed** response crosses the wire while the callee is still producing it, so the call takes `first byte + size / the slowest stage` (the store, a node's card or the wire): the pipeline limit, cut-through as chunks get small. The transfer starts at the callee's **first byte** (the end of its last own hop less that hop's size-dependent part: a store's `first_byte`, a database op's time, a service's `proc`) and shares the wire fairly with other transfers while it is on it. **A read of a `storage` node streams its response by default**, as S3, GCS and Azure send a GET as it is read; everything else is buffered. `stream: true` (both directions), `stream: false` (neither), `stream: response` or `stream: request` on an edge or a `call:` step changes that (a step beats its edge). A callee whose sequence ends in another call cannot send early, so it stays as slow as buffered; a call with no response bytes or no pipe with a rate has nothing to overlap, so it is unchanged; a failing callee sends nothing. In the trace a streamed call is a fork: the callee's hops beside a `first-byte` wait and the response `link`. **An upload streams too:** a **write** to a `storage` node streams its request by default, as a store takes a PUT as the bytes arrive: the request crossing the wire and the store's work run side by side (a fork in the trace), so a 1 MiB PUT over a 1 MiB/s wire to a store that needs 30 ms takes the wire's 1 s, not 1.03 s. `stream: request` does the same for any callee (a service that consumes an upload as it arrives). The response, if it also streams, starts from the callee's first byte as before; a large response to a streamed upload starts when the callee ends, not when the upload does. Not modelled: chunk-level store-and-forward for small bodies, a client that reads slowly (back-pressure), TCP window limits between the stages.

**Packet loss: `loss`.** `loss: 0.01` on an edge says one packet in a hundred is lost; the default is none, so nothing moves until you write it. A lost packet costs TCP a recovery, and the model takes the published one: a body of up to 32 expected losses (`segments x loss`) draws its lost segments (binomial) and pays, for each, **one round trip** when three or more segments follow it (fast retransmit, RFC 5681) or the **retransmission timeout** when they do not (the tail of the body, or a body of a few segments; `max(200 ms, 3 x round trip)`, RFC 6298; the chance is `min(1, 3 / flight)`, Padhye et al. 1998). A longer flow runs at the **steady TCP rate of RFC 5348** (the PFTK equation) and the call pays the time that takes over the lossless time. A new connection's handshake packets are lost the same way, and a lost SYN waits **1 s** (RFC 6298). The cost is in round trips, so **an edge with no `net` pays nothing**. It shows in the log as `retransmit` and `retransmit response` hops. Not modelled: a retransmission lost again, the window halving after a recovered loss inside a short flow, SACK, ECN, reordering.

**Compression: `compress`.** `compress: 0.3` on an edge sends 30% of a response's bytes over the wire (gzip or brotli on JSON is commonly 0.1 to 0.3; already compressed media is about 1). The wire, slow start and loss see the smaller body; the component still handles the whole one, so a store's `throughput` and a node's `per_kb` are unchanged (compressing costs the compressing node's own `per_kb`). Only a response is compressed, and headers are not. Default none; there is no universal ratio, so the number is yours.

**Units for sizes and rates.** Every byte field takes a whole number (bytes, or bytes per second for a rate: unchanged from before) or text with a unit: `250KB`, `250 KB`, `1.5MB`, `2GB`, `512B`. Units are `B`, `KB`, `MB`, `GB` in any case, with `KiB`, `MiB`, `GiB` as the same thing, and **KB = 1024 bytes** (the same as Studio's unit picker, so a number means the same in the file and on the page). A fraction rounds to the nearest whole byte. A rate (node and edge `bandwidth`, the `bandwidth_cap` of a scenario or the control API) may also end in `/s`: `1MB/s`, `250kb/s`; a size (`req_bytes`, `resp_bytes` in [traffic](traffic.md)) refuses `/s`. A negative number, a missing number (`KB`), `NaN`, an unknown unit (`5XB`) or a value past 2^64 bytes is a load error such as ``kaanal.yaml:6:40: edge #0: field 'bandwidth': bandwidth: unknown unit in `5XB`; use B, KB, MB, GB, optionally /s for a rate``. The bounds (bandwidth at least 1 byte a second, sizes at most 1 GiB) apply after the conversion, so `bandwidth: 0KB` fails like `0`. No quotes are needed (`bandwidth: 250KB` and `bandwidth: 1MB/s` are plain YAML); in the control API's JSON the value is a string. Studio shows the same number for `250KB` and `256000` and keeps the text when it saves a file it did not change; an edit to the field writes whole bytes.

**Network card (`bandwidth` on a node).** Any node except a cache takes `bandwidth:` as a rate (at least 1 byte per second): one number for both directions, or `{ out: 125000, in: 250000 }` (either side may be left out; a missing side is unlimited). It is the node's network card, so it is shared by every wire in that direction: calls to different services from one node share its `out`, and the replies it sends share it too. Each copy has its own send and receive pipe, so more copies mean more bandwidth. A transfer from A to B is pipelined: it lasts `bytes / the narrowest of A's out, the wire's bandwidth and B's in`, when it is alone. Pipes are **shared fairly** (max-min, as TCP converges to): every pipe splits its rate equally between the transfers crossing it, and a transfer held back by a slower pipe leaves its spare share to the others, so a big upload no longer blocks a small call and a fan-out through one card finishes together. The time a transfer loses to sharing is its wait in the trace and the Log. Limits of this model: a share is floored to whole bytes per second (under 1 B/s of error), a transfer has no cap of its own (a TCP window is planned), there is no connection set-up time, and the live mock does not share a pipe between real requests. A load balancer is just a node with a `bandwidth`. In a trace a transfer held up by a card shows as `app.nic-out=20.0ms` or `app.nic-in=20.0ms` (its time alone, the wait showing what sharing added), the node being the one whose card it is. Unset means no limit and no hop. The live mock sizes the request from `Content-Length`, or, when there is none (a chunked upload) and the route does not read the body itself, from the bytes the client really sends (counted as they arrive, up to 1 GiB, and thrown away); a login or a collection route that reads its own body counts 0 without a `Content-Length`. It counts a HEAD response and a collection or built-in responder as 0 bytes, and sizes the normal success body even when a pin replaces it.

## What a slot covers: `hold`

A gateway, service, monolith or auth node with `concurrency` can say what one of its slots is held for:

| `hold`          | A slot is held                                                                       | Fits                                                                 |
| --------------- | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| `own` (default) | Only while the node does its own work (its `proc`)                                   | An event-loop server that waits on its calls: Node, Go, async Python |
| `call`          | From the moment the request is admitted until everything the node calls has answered | A thread or worker per request: Tomcat, sync Gunicorn, Puma, PHP-FPM |

With `call`, a slow dependency fills the slots of the node above it, its queue grows and it turns requests away, as such a server does, so a slow database shows up as a busy, saturated app instead of an app that looks idle. The node's own hops run in the slot it holds, and its response transfer (a wire's `bandwidth`) happens after the slot is let go.

```yaml
nodes:
  app: { type: service, concurrency: 16, proc: 10ms, hold: call }
```

- A node with no `concurrency` has no slot to hold, so `hold` changes nothing for it. A database, a cache and an external API call nothing, so they do not take the field.
- A call that fails or times out frees the caller's slot at that moment, as a worker is freed when its call errors; a node whose own hop is still running keeps the slot until that hop ends.
- A held node's offered load, busy share per window and saturation point are measured from the time its slots were held, so they include the wait on what it called. They move with the dependency: when the database slows down, the app's "breaks first" rate falls.
- Studio: the **Server style** choice on the Controls tab, for any component that calls others: **Event loop (default)** writes nothing (`hold: own`); **Thread per request** writes `hold: call` and, on a component with no slot limit yet, also `concurrency: { limit: 200, queue: 100 }` (Tomcat's defaults; change them under Capacity). Choosing the event loop again removes `hold` and leaves the slots as they are.

**Not built yet:**

- `hold` is per node. There is no hold per wire or per endpoint, so a node that holds its slot for one dependency but not another cannot say so.
- A caller that carries on after a failed call, and retries, are not modelled yet: a failing call ends the request (continue-after-failure and retries are a later step).
- A held node's response transfer is outside the hold, so a very large response over a slow wire frees the slot a little earlier than a real thread-per-request server would.

## Replicas and balancing

Any node can have copies: `replicas: 3` (1 to 64, default 1). Each replica has its own slots, queue, pool and rate limit, so three replicas of a service with `concurrency: 4` serve twelve requests at once. `balance` says how traffic entering the node picks a replica:

| `balance`                 | Picks                                                                                                                          |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `round-robin` (default)   | The next live replica in turn                                                                                                  |
| `random`                  | A uniformly random live replica                                                                                                |
| `least-conn`              | The live replica with the fewest requests in it (busy slots plus those waiting for one) when the call is sent; ties take turns |
| `p2c`                     | Power of two choices: two random live replicas, the less loaded of the two when the call is sent; a tie takes the first        |
| `{ weighted: [3, 1, 1] }` | A random live replica, weighted: one weight per replica, at least one above 0                                                  |

`least-conn` and `p2c` read the live queues, so the **simulator** (Studio runs, `kaanal-sim`) chooses at the moment each call is sent, as a real balancer does; the **live mock** has no queues to look at and plays both as round robin (least-conn) or a random live replica (p2c) until live-load mode. Two cautions: the simulator reads exact queue lengths, so it is a little kinder than a real balancer with delayed counts; and chaos aimed at one replica works with them for `down` and for added latency, but an error rate set on one replica is not applied (the node-wide chaos is). A balancer's `spread` cannot use them (a load error says so): put `least-conn` or `p2c` on the `balance` of the components behind the balancer.

A node sets `balance` for traffic entering it, and an edge may override it for calls over that edge: `{ from: app, to: db, balance: random }`. A weighted list must have exactly one weight per replica of the callee. Without `replicas`, `balance` does nothing. Database replicas all take every request: there is no read/write split yet, so a write is not sent to a primary only.

Chaos can target one replica: `kaanal node down app --replica 1`, `kaanal chaos set app --replica 2 --latency 50ms`, `kaanal chaos clear app --replica 2` (scenario entries and the API take `replica` next to `node`). A downed replica is skipped by the balancer; the other copies keep serving, and a node with every replica down answers 503. Output names a replica `app#1`.

A request's trace names the copy that served each hop (`db#2.read=4.1ms`, and `db#2.read` in `kaanal logs` waterfalls), the logged hop carries `replica`, `GET /__kaanal/topology` lists `replicas` on a replicated node, and `kaanal node list` shows `db x3`. Chaos latency is attributed to the replica it hit.

Two details. A cache never fails a request, so a cache with every replica down is a miss, as a down cache already was. And when a node is down, a call to it with more than one replica fails at the call start (no network hop, no timeout span), while a call to a one-replica node fails at the callee's own hop, after the network hop: the same 503, a different trace.

## The gateway is optional

With no gateway, a request enters at the `service` or `monolith` that owns the route in its `spec`. With a gateway, every node that has a `spec` needs an edge from the gateway. Static endpoints (an `endpoints:` entry with `override` and no owning service) need a gateway.

See `examples/gatewayless` in the repository for a complete gateway-less file.

## Presets fill what you leave out

A `flavor` fills the values you do not write; anything you write wins. **All numbers below are estimates**: typical single-node figures for a warm instance with small rows, network excluded (edges carry `net`).

| Flavor     | read (PK lookup) p50 / p99 | write (single-row commit) p50 / p99 | scan (indexed list, ~100 rows) p50 / p99 |
| ---------- | -------------------------- | ----------------------------------- | ---------------------------------------- |
| `postgres` | 0.3 ms / 2 ms              | 1 ms / 8 ms                         | 3 ms / 25 ms                             |
| `mysql`    | 0.3 ms / 2.5 ms            | 1.2 ms / 10 ms                      | 3 ms / 30 ms                             |
| `mongodb`  | 0.4 ms / 3 ms              | 0.8 ms / 6 ms                       | 4 ms / 35 ms                             |

| Flavor      | `hit_ratio` | hit p50 / p99    | miss p50 / p99   |
| ----------- | ----------- | ---------------- | ---------------- |
| `redis`     | 0.9         | 0.2 ms / 1 ms    | 0.2 ms / 1 ms    |
| `memcached` | 0.9         | 0.15 ms / 0.8 ms | 0.15 ms / 0.8 ms |

## Every error names file, line, column, node and field

```text
kaanal.yaml:12:5: node 'orders-db': unknown field 'bogus' (expected: balance, bandwidth, concurrency, flavor, locale, ops, per_kb, pool, rate_limit, replicas, timeout)
```

Run `kaanal schema` to print a JSON Schema of the whole file for editor autocomplete.
