---
title: "Nodes reference"
sidebar:
  order: 3
---

Every node kind you can put under `nodes:`, its fields, and what each field does today. Set the kind with `type:`. A field that does not belong to a kind is rejected when the file loads.

Fields marked **active now** change the simulation. Fields marked **not active yet** are accepted and checked (a bad value is still an error) so your file will not need rewriting later, but they have no effect on timing yet.

How a request walks these nodes, and how to change the walk per route, is in the [endpoints reference](/docs/guides/mock/endpoints/).

Durations are `5ms`, `{ p50: 4ms, p99: 20ms }` or `{ mean: 5ms }`, as described in the [user guide](/docs/start/install/).

## `gateway`

A reverse proxy or API gateway (nginx, Envoy). Optional, at most one. For a balancer that spreads calls over several components, see [`load-balancer`](#load-balancer). With a gateway, every request pays its `proc` first, then runs its auth calls, and it answers static endpoints (an `endpoints:` entry with `override` and no owning service).

```yaml
nginx: { type: gateway, flavor: nginx, proc: 1ms }
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ---------- |
| `flavor`      | Free-text label shown in status output (`nginx`, `envoy`)                                                                                                                                                                                              | none      | active now |
| `proc`        | Processing time per request                                                                                                                                                                                                                            | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now |

## `service`

A backend service. It may carry an OpenAPI 3.0 `spec`. Without a gateway, the routes in its `spec` are served here.

```yaml
app: { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                                                                                                                          | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                                                                                                                                 | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now |

## `monolith`

A service that can also be its own database. With `ops` or a `flavor` it has inline read/write/scan costs; with edges to databases it behaves as a service. Both together are allowed.

```yaml
core: { type: monolith, spec: openapi.yaml, proc: 3ms, flavor: mysql, ops: { read: 1ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                                                                                                                          | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                                                                                                                                 | zero      | active now |
| `ops`         | Inline `read`, `write`, `scan` costs; any you leave out comes from `flavor`                                                                                                                                                                            | none      | active now |
| `flavor`      | Preset that fills `ops` you left out: `postgres`, `mysql`, `mariadb`, `sqlserver`, `oracle`, `mongodb`, `neo4j`, `cassandra`, `dynamodb`                                                                                                               | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now |

## `database`

A data store with costs for `read`, `write` and `scan`. Give all three, or a `flavor` to fill the gaps.

```yaml
db: { type: database, flavor: postgres, ops: { write: 9ms } }
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ---------- |
| `ops`         | `read`, `write`, `scan` costs; required in full unless `flavor` fills them                                                                                                                                                                             | none      | active now |
| `flavor`      | Preset for ops you left out: `postgres`, `mysql`, `mariadb`, `sqlserver`, `oracle`, `mongodb`, `neo4j`, `cassandra`, `dynamodb`                                                                                                                        | none      | active now |
| `pool`        | Connection pool: `size` (at least 1), `queue` (default 0), `overflow_status` (500 to 504, default 503)                                                                                                                                                 | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now |

## `cache`

A cache in front of slower nodes, with a hit ratio and the cost of a hit and of a miss. Without a `flavor`, `hit_ratio`, `hit` and `miss` are all required.

```yaml
cache: { type: cache, flavor: redis, hit_ratio: 0.8 }
```

| Field         | Meaning                                                                                                      | Default     | Status                               |
| ------------- | ------------------------------------------------------------------------------------------------------------ | ----------- | ------------------------------------ |
| `flavor`      | Preset for `hit_ratio`, `hit` and `miss`: `redis`, `memcached`, `valkey`                                     | none        | active now                           |
| `hit_ratio`   | Share of lookups that hit, 0 to 1                                                                            | from flavor | active now                           |
| `hit`         | Cost of a hit                                                                                                | from flavor | active now                           |
| `miss`        | Cost of finding out it is a miss                                                                             | from flavor | active now                           |
| `ttl`         | Entry lifetime                                                                                               | none        | accepted and checked, not active yet |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                              | unlimited   | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429 | none        | active now                           |

A lookup slower than the cache's `timeout` counts as a miss. If `timeout` is below the `hit` cost (its median, for a distribution), most hits are cut, and loading prints a warning naming the cache.
| `timeout` | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none | active now |

## `external-api`

A third-party HTTP API such as payments or email.

```yaml
pay: { type: external-api, proc: 50ms, error_rate: 0.01 }
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ------------------------------------ |
| `proc`        | Response time of the API                                                                                                                                                                                                                               | zero      | active now                           |
| `error_rate`  | Baseline share of failing calls, 0 to 1                                                                                                                                                                                                                | none      | accepted and checked, not active yet |
| `proxy`       | Real upstream URL (`http://` or `https://`)                                                                                                                                                                                                            | none      | accepted and checked, not active yet |
| `pool`        | Connection pool, as on `database`                                                                                                                                                                                                                      | none      | active now                           |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now                           |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now                           |

## `auth`

An identity provider: serves `POST /login` and `GET /.well-known/jwks.json` (see [Auth](/docs/guides/mock/auth/)). At most one.

```yaml
login:
  type: auth
  proc: 5ms
  token_ttl: 2h
  claims: { aud: my-app }
  users: [{ username: alice, password: secret, roles: [admin], scopes: [read] }]
```

| Field         | Meaning                                                                                                                                                                                                                                                | Default   | Status     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------- | ---------- |
| `proc`        | Cost of one login, one JWKS request or one gateway token check                                                                                                                                                                                         | zero      | active now |
| `users`       | Accounts that can log in (`username` unique, `password`, `roles`, `scopes`, `claims`)                                                                                                                                                                  | none      | active now |
| `token_ttl`   | Lifetime of issued tokens, whole seconds, 1s to 1 day                                                                                                                                                                                                  | 1h        | active now |
| `claims`      | Extra claims in every token (a user's own `claims` win)                                                                                                                                                                                                | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                                                                                                        | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                                                                                                           | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. When it passes the caller gets a 504 (`timeout`); work already running keeps its slot until it ends. Above 0, at most `5m`; see [Chaos](/docs/how-it-works/#chaos) | none      | active now |

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

| Field                                                   | Meaning                                                                                                                                                                                                                                                                                                                                | Default            |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `first_byte`                                            | Time to the first byte of a call, the node's service time. A duration like `proc`: `25ms`, `{ p50: 20ms, p99: 80ms }` or `{ mean: 25ms }`. **Optional**: absent means 25 ms, a labelled estimate (no provider publishes one), so write your own measurement                                                                            | none               |
| `throughput`                                            | Bytes per second the store moves for one call: a whole number or text like `80MB/s`, `250KB/s` (KB = 1024 bytes, see [units](/docs/reference/resources/#payload-cost)); at least 1 byte per second. A larger body takes longer                                                                                                         | none: no size term |
| `class`, `retrieved`                                    | The storage class and the data read back in a month, for the **cost estimate only** (below): `standard` (default), `infrequent`, `cold` or `archive`; `retrieved` is a size (`50GB`). Optional                                                                                                                                         | `standard`, none   |
| `stored`, `write_share`                                 | The data kept and the share of writes: inputs of the **cost estimate only** (below). Optional                                                                                                                                                                                                                                          |                    |
| `location`                                              | Where the store is priced, for the **cost estimate only**: a region of its cloud (`us-east-1`, `us-east`, `us-central1`) or, for Google, a dual-region (`nam4`, `eur4`, `eur5`, `eur7`, `eur8`, `asia1`) or multi-region (`us`, `eu`, `asia`). Absent = the region picked at the globe. A name with no price is refused, with the path | the globe's region |
| `flavor`                                                | `s3`, `gcs` or `blob`. A **label for timing** (it sets no number there); it also picks the cloud the **cost estimate** prices the store in (shown on the page and in status output); it sets no number                                                                                                                                 | none               |
| `concurrency`                                           | A number, or `{ limit, queue, overflow_status }`: calls served at once                                                                                                                                                                                                                                                                 | unlimited          |
| `rate_limit`                                            | Token bucket: `rps` and `burst`; over it a call fails with **503** (S3's "Slow Down"), not 429 like the other kinds                                                                                                                                                                                                                    | none               |
| `timeout`, `replicas`, `balance`, `bandwidth`, `locale` | As on the other nodes                                                                                                                                                                                                                                                                                                                  |                    |

**What a store costs (the cost estimate, by region).** Two optional fields feed the estimate and change **no timing**: `stored` (the data kept, `500GB`, `2TB`) and `write_share` (the share of calls that are writes, 0 to 1; absent = all reads). The card prices the store the way every object store bills: **the data kept per GB-month** (over 730 hours an hour) **plus the requests**, a `write_share` of them at the write price (PUT, COPY, POST, LIST) and the rest at the read price (GET), at the visit rate of the run on screen: **the live rate** (the last five seconds) in Live and Admin, the **last batch run's** otherwise (so request cost shows once traffic runs, the data cost at once). Copies do not multiply it: one store, one bill. **A colder `class`** keeps data for less and charges more to use it: dearer requests, and a **fee for each GB read back** (`retrieved`, a size per month, priced over the month's hours). `infrequent` is S3 Standard-IA, Azure Cool and Google Nearline; `cold` is S3 Glacier Instant Retrieval, Azure Cold and Google Coldline; `archive` is S3 Glacier Flexible Retrieval (standard retrieval), Azure Archive and Google Archive. Examples in us-east-1: 1,024 GB in Standard is about $0.032 an hour; in Standard-IA with 100 GB read back a month it is about $0.019. A class a region does not sell has no price there (the card says so). Not in it: Glacier Deep Archive (AWS prices it in a separate offer file), One Zone-IA, Intelligent-Tiering, minimum-duration and early-deletion charges, and expedited or bulk retrieval.

| `flavor`     | Priced as                                                                                | Source                                                                                           | Regions                                                                                                                                                                                                                                                                                                   |
| ------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `s3` or none | AWS S3 Standard, first tier                                                              | AWS's public price list                                                                          | 36 regions, each its own price (us-east-1: $0.023 a GB-month, $0.005 per 1,000 writes, $0.0004 per 1,000 reads)                                                                                                                                                                                           |
| `blob`       | Azure Blob, Hot, LRS                                                                     | Azure's public retail prices API                                                                 | 56 regions (Azure UK, Jio, Saudi Arabia and government regions have no match in the region list and are left out)                                                                                                                                                                                         |
| `gcs`        | Google Cloud Storage Standard, single regions and the predefined dual- and multi-regions | Google's pricing page, read in a browser on 2026-11-12 (`scripts/gcs-storage.csv`, kept by hand) | 46 regions, each its own price (us-central1: $0.02 a GB-month, $0.005 per 1,000 writes, $0.0004 per 1,000 reads), plus `nam4` $0.044 (it bills both of its regions), `eur4` $0.044, `asia`, `eu` and `us` $0.026, with $0.01 per 1,000 writes in each of those (a configurable dual-region is not priced) |

The region is the one you pick at the globe, and the price follows it. Left out of the estimate: data transfer out, retrieval fees, other storage classes (Infrequent Access, Glacier, Archive), lifecycle and replication. Refresh with `node scripts/gen-storage-prices.mjs`; the table is `crates/kaanal-core/data/storage-prices.csv`, served at `GET /__kaanal/storage-prices`.

**Cost of one call** is `first_byte + (request bytes + response bytes) / throughput`, with KB = 1024. The bytes are the request and response sizes already in the traffic (a stream's `req_bytes` and `resp_bytes`; the live mock's `Content-Length` and body size), so there is no size field of its own; with no sizes, or no `throughput`, the size term is nothing. Example: `first_byte: 20ms`, `throughput: 100MB/s` and a 5 MB response cost 20 ms + 5 MiB / (100 MiB/s) = 20 ms + 50 ms = 70 ms; a 5 KB object adds under a millisecond. It runs on the same machinery as `proc` and `per_kb`: the node's service time is `first_byte` and the size term is `per_kb` worked out as `1024 / throughput` seconds per KB (held to the nanosecond, so a size term is exact to about 1 part in 10^4 at 100 MB/s). A storage node therefore refuses `proc` and `per_kb` (use `first_byte` and `throughput`), and `ops`, `pool`, `hit_ratio` and `hold` (it calls nothing, and its concurrency is `concurrency`), each as a load error that says so.

**Operations.** A step or route `op: read`, `write` or `scan` toward a storage node is accepted exactly as for a database, and means `GET`, `PUT` and `LIST`. All three cost the same `first_byte` plus the size term per request (a read, `GET`, also **streams** over a wire or card with a `bandwidth`, see below): no provider publishes a different first-byte latency for a listing, so there is no separate LIST number. What makes a listing cost more is that it **pages**: S3 returns at most 1,000 keys per `LIST` request (GCS and Azure Blob page the same way), so a listing of N keys is about N / 1,000 requests one after another, each paying `first_byte` and its own page. Write that as `- { call: assets, op: scan, repeat: 10, returns: 200KB }`: ten pages of the size one page of your keys takes (it depends on key length, so measure it). A bare `call: assets` works, and a service's default body calls its storage nodes after its database-style callees. Whether a storage call holds the caller's slot follows the caller's own `hold` setting; the node holds one of its own slots only for its own work.

**Rate limit, and why there are no presets.** `rate_limit` is the node's request rate (one limit per node; the simulator has no key prefixes). It is off unless you set it and the flavor does not set it. No provider publishes a first-byte latency for standard object storage, so **there are no presets**: `first_byte` and `throughput` are numbers you write from your own measurements. The request-rate figures that are published (searched 2026-11-04) are:

- AWS S3: at least 3,500 PUT, COPY, POST and DELETE requests and 5,500 GET and HEAD requests per second per prefix, and a sizing guideline of one request per 85 to 90 MB/s; it answers 503 Slow Down while it scales (docs.aws.amazon.com/AmazonS3/latest/userguide/optimizing-performance-design-patterns.html).
- Google Cloud Storage: about 5,000 object reads and 1,000 object writes per second per bucket to start, scaling as it goes; it exposes no latency metric (docs.cloud.google.com/storage/docs/request-rate).
- Azure Blob: up to 3,000 requests per second for one block blob; throughput is bounded by the storage account (learn.microsoft.com/azure/storage/blobs/scalability-targets).

**A storage node in Studio** starts as `{ type: storage, flavor: s3, first_byte: 25ms, throughput: 80MB/s }`. Those two numbers are **placeholders to edit, not published figures** (see above). It has no `concurrency` on purpose: the request-rate figures belong to `rate_limit`. Studio edits every field above in boxes (see the Storage entry of [Studio](/docs/guides/simulate/studio/)); its **Add a request limit** button writes `rate_limit: { rps: 5500, burst: 5500 }`, S3's published read rate per prefix, as a starting point to change (every other kind's button starts at `{ rps: 1000, burst: 1000 }`, a placeholder, because no provider publishes a figure for a service or a database).

## Nodes added in Studio

`add_node` (see the [control API](/docs/reference/control-api/draft/)) writes a node that is valid as it stands and has a capacity, so the bottleneck report can rank it. These are **estimates**: typical sizes for a small deployment, meant to be edited.

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
