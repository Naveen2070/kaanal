---
title: "Nodes reference"
sidebar:
  order: 3
---

Every node kind you can put under `nodes:`, its fields, and what each field does today. Set the kind with `type:`. A field that does not belong to a kind is rejected when the file loads.

Fields marked **active now** change the simulation. Fields marked **not active yet** are accepted and checked (a bad value is still an error) so your file will not need rewriting later, but they have no effect on timing yet.

How a request walks these nodes, and how to change the walk per route, is in the [endpoints reference](/docs/guides/mock/endpoints/).

Durations are `5ms`, `{ p50: 4ms, p99: 20ms }` or `{ mean: 5ms }`, as described in the [user guide](/docs/start/install/).

## Common fields

Several kinds share these fields. The per-kind tables below repeat them in short.

| Field         | Detail                                                                                                                                            |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`. Default unlimited                                                |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1). Over it a request fails with 429                                      |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`. See [Chaos](/docs/how-it-works/#chaos) |

When a `timeout` passes, the caller gets a 504 (`timeout`). Work that is already running keeps its slot until it ends.

## `gateway`

A reverse proxy or API gateway (nginx, Envoy). Optional, at most one. For a balancer that spreads calls over several components, see [`load-balancer`](#load-balancer). With a gateway, every request pays its `proc` first, then runs its auth calls, and it answers static endpoints (an `endpoints:` entry with `override` and no owning service).

```yaml
nginx: { type: gateway, flavor: nginx, proc: 1ms }
```

| Field         | Meaning                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `flavor`      | Free-text label shown in status output (`nginx`, `envoy`)                                                                                      | none      | active now |
| `proc`        | Processing time per request                                                                                                                    | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now |

## `service`

A backend service. It may carry an OpenAPI 3.0 `spec`. Without a gateway, the routes in its `spec` are served here.

```yaml
app: { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
```

| Field         | Meaning                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                  | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                         | zero      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now |

## `monolith`

A service that can also be its own database. With `ops` or a `flavor` it has inline read/write/scan costs; with edges to databases it behaves as a service. Both together are allowed.

```yaml
core: { type: monolith, spec: openapi.yaml, proc: 3ms, flavor: mysql, ops: { read: 1ms } }
```

| Field         | Meaning                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                                                                                  | none      | active now |
| `proc`        | Processing time per request, charged wherever a chain reaches the node                                                                         | zero      | active now |
| `ops`         | Inline `read`, `write`, `scan` costs; any you leave out comes from `flavor`                                                                    | none      | active now |
| `flavor`      | Preset that fills `ops` you left out: `postgres`, `mysql`, `mariadb`, `sqlserver`, `oracle`, `mongodb`, `neo4j`, `cassandra`, `dynamodb`       | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now |

## `database`

A data store with costs for `read`, `write` and `scan`. Give all three, or a `flavor` to fill the gaps.

```yaml
db: { type: database, flavor: postgres, ops: { write: 9ms } }
```

| Field         | Meaning                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `ops`         | `read`, `write`, `scan` costs; required in full unless `flavor` fills them                                                                     | none      | active now |
| `flavor`      | Preset for ops you left out: `postgres`, `mysql`, `mariadb`, `sqlserver`, `oracle`, `mongodb`, `neo4j`, `cassandra`, `dynamodb`                | none      | active now |
| `pool`        | Connection pool: `size` (at least 1), `queue` (default 0), `overflow_status` (500 to 504, default 503)                                         | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now |

## `cache`

A cache in front of slower nodes, with a hit ratio and the cost of a hit and of a miss. Without a `flavor`, `hit_ratio`, `hit` and `miss` are all required.

```yaml
cache: { type: cache, flavor: redis, hit_ratio: 0.8 }
```

| Field         | Meaning                                                                                                                                                           | Default     | Status                               |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------- | ------------------------------------ |
| `flavor`      | Preset for `hit_ratio`, `hit` and `miss`: `redis`, `memcached`, `valkey`                                                                                          | none        | active now                           |
| `hit_ratio`   | Share of lookups that hit, 0 to 1                                                                                                                                 | from flavor | active now                           |
| `hit`         | Cost of a hit                                                                                                                                                     | from flavor | active now                           |
| `miss`        | Cost of finding out it is a miss                                                                                                                                  | from flavor | active now                           |
| `ttl`         | Entry lifetime                                                                                                                                                    | none        | accepted and checked, not active yet |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                                   | unlimited   | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                                      | none        | active now                           |
| `timeout`     | Deadline for a call to this node (`> 0`, max `5m`; see [Chaos](/docs/how-it-works/#chaos)). Lookups slower than `timeout` count as misses (warns if below `hit`). | none        | active now                           |

## `external-api`

A third-party HTTP API such as payments or email.

```yaml
pay: { type: external-api, proc: 50ms, error_rate: 0.01 }
```

| Field         | Meaning                                                                                                                                        | Default   | Status                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ------------------------------------ |
| `proc`        | Response time of the API                                                                                                                       | zero      | active now                           |
| `error_rate`  | Baseline share of failing calls, 0 to 1                                                                                                        | none      | accepted and checked, not active yet |
| `proxy`       | Real upstream URL (`http://` or `https://`)                                                                                                    | none      | accepted and checked, not active yet |
| `pool`        | Connection pool, as on `database`                                                                                                              | none      | active now                           |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now                           |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now                           |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now                           |

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

| Field         | Meaning                                                                                                                                        | Default   | Status     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------- | ---------- |
| `proc`        | Cost of one login, one JWKS request or one gateway token check                                                                                 | zero      | active now |
| `users`       | Accounts that can log in (`username` unique, `password`, `roles`, `scopes`, `claims`)                                                          | none      | active now |
| `token_ttl`   | Lifetime of issued tokens, whole seconds, 1s to 1 day                                                                                          | 1h        | active now |
| `claims`      | Extra claims in every token (a user's own `claims` win)                                                                                        | none      | active now |
| `concurrency` | A number (most requests served at once), or `{ limit, queue, overflow_status }`                                                                | unlimited | active now |
| `rate_limit`  | Token bucket: `rps` (above 0, at most 1000000000) and `burst` (at least 1); over it a request fails with 429                                   | none      | active now |
| `timeout`     | Deadline for a call to this node: queue wait, its own work and everything it calls. Above 0, at most `5m`; see [Common fields](#common-fields) | none      | active now |

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

- **Target selection**: the chosen component picks a copy via its own `balance` while its wire keeps `net`, `calls`, and `bandwidth`. Edges out of a balancer cannot set `prob`, and total callee outage returns `503` at the balancer. Round-robin counts per balancer copy so copies do not lock to callees.
- **Routes behind balancers**: if the gateway has no direct wire to a route's owner but reaches it through one or more balancers, traffic crosses each balancer (path-based routing: the route decides the target, not `spread`, paying each balancer's time, slots, queue, `bandwidth`, and `hold`). A direct gateway wire wins if present. Use `replicas` + `balance` on one service for identical pools; use `spread` to pick between different components.
- **Not built yet**: `least-conn` and `p2c` on `spread` (use `balance` on the target components instead).

## `storage`

An object store (S3, Google Cloud Storage, Azure Blob): a **leaf** that calls nothing. Whatever may call a database may call it (`service`, `monolith`, `auth`, and a `load-balancer` in front of it).

```yaml
assets:
  type: storage
  flavor: s3
  first_byte: 25ms
  throughput: 80MB/s
```

| Field                                                   | Meaning                                                                                                             | Default            |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `first_byte`                                            | Time to the first byte of a call, the node's service time. Optional                                                 | none               |
| `throughput`                                            | Bytes per second the store moves for one call. A larger body takes longer                                           | none: no size term |
| `class`, `retrieved`                                    | Storage class and data read back in a month, for the **cost estimate only** (below). Optional                       | `standard`, none   |
| `stored`, `write_share`                                 | The data kept and the share of writes: inputs of the **cost estimate only** (below). Optional                       |                    |
| `location`                                              | Where the store is priced, for the **cost estimate only**                                                           | the globe's region |
| `flavor`                                                | `s3`, `gcs` or `blob`. A **label for timing** (it sets no number there)                                             | none               |
| `concurrency`                                           | A number, or `{ limit, queue, overflow_status }`: calls served at once                                              | unlimited          |
| `rate_limit`                                            | Token bucket: `rps` and `burst`; over it a call fails with **503** (S3's "Slow Down"), not 429 like the other kinds | none               |
| `timeout`, `replicas`, `balance`, `bandwidth`, `locale` | As on the other nodes                                                                                               |                    |

Field notes:

- **`first_byte`**: a duration like `proc`: `25ms`, `{ p50: 20ms, p99: 80ms }` or `{ mean: 25ms }`. Absent means 25 ms, a labelled estimate (no provider publishes one), so write your own measurement.
- **`throughput`**: a whole number or text like `80MB/s`, `250KB/s` (KB = 1024 bytes, see [units](/docs/reference/resources/#payload-cost)); at least 1 byte per second.
- **`class`**: `standard` (default), `infrequent`, `cold` or `archive`. **`retrieved`** is a size (`50GB`).
- **`location`**: a region of its cloud (`us-east-1`, `us-east`, `us-central1`) or, for Google, a dual-region (`nam4`, `eur4`, `eur5`, `eur7`, `eur8`, `asia1`) or multi-region (`us`, `eu`, `asia`). Absent = the region picked at the globe. A name with no price is refused, with the path.
- **`flavor`**: also picks the cloud the **cost estimate** prices the store in (shown on the page and in status output); it sets no number.

### Cost estimate (by region)

`stored` (`500GB`, `2TB`) and `write_share` (`0`–`1`, default `0`) feed the hourly cost estimate without affecting timing. Billing sums:

- **Data kept**: per GB-month (÷ 730 hours).
- **Requests**: `write_share` at write price for PUT/COPY/POST/LIST, the rest at read price for GET, using the active run's visit rate (copies do not multiply it).

- **Storage classes (`class`, `retrieved`)**: `infrequent` (S3 Standard-IA, Azure Cool, GCS Nearline), `cold` (S3 Glacier Instant, Azure Cold, GCS Coldline), and `archive` (S3 Glacier Flexible, Azure Archive, GCS Archive) charge less per GB kept, more per request, and a per-GB retrieval fee (`retrieved` per month ÷ 730). Unsupported region/class pairs show no price. (Excludes Glacier Deep Archive, One Zone-IA, Intelligent-Tiering, early-deletion/minimum-duration fees, and expedited/bulk retrieval.)

| `flavor`     | Priced as                    | Source & regions                                                                                                                                                                                                                    |
| ------------ | ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `s3` or none | AWS S3 Standard (first tier) | AWS price list, `36` regions (`us-east-1`: `$0.023`/GB-mo, `$0.005`/1k writes, `$0.0004`/1k reads)                                                                                                                                  |
| `blob`       | Azure Blob Hot LRS           | Azure retail API, `56` regions (excludes UK, Jio, Saudi Arabia, gov)                                                                                                                                                                |
| `gcs`        | GCS Standard                 | Google pricing page (`scripts/gcs-storage.csv`), `46` regions (`us-central1`: `$0.02`/GB-mo, `$0.005`/1k writes, `$0.0004`/1k reads), plus `nam4` (`$0.044`), `eur4` (`$0.044`), and `asia`/`eu`/`us` (`$0.026`, `$0.01`/1k writes) |

Refreshed via `node scripts/gen-storage-prices.mjs` (`crates/kaanal-core/data/storage-prices.csv`, served at `GET /__kaanal/storage-prices`). Data transfer out, lifecycle, and replication are omitted.

### Timing, operations & rate limits

- **Call cost**: `first_byte + (request bytes + response bytes) / throughput` (`1 KB = 1024 B`; uses traffic `req_bytes`/`resp_bytes` or live `Content-Length`/body size; `0` size term if sizes or `throughput` are unset). Example: `first_byte: 20ms`, `throughput: 100MB/s` and a 5 MB response cost 20 ms + 5 MiB / (100 MiB/s) = 70 ms; a 5 KB object adds under a millisecond. The size term is `per_kb` worked out as `1024 / throughput` seconds per KB (exact to about 1 part in 10^4 at 100 MB/s). Storage nodes refuse `proc`, `per_kb`, `ops`, `pool`, `hit_ratio`, and `hold`.
- **Operations (`read`, `write`, `scan`)**: `GET`, `PUT`, and `LIST` all cost `first_byte` + size term (`GET` streams over `bandwidth` edges). S3/GCS/Blob `LIST` pages at most `1,000` keys per call, so model multi-page listings with `repeat`: `- { call: assets, op: scan, repeat: 10, returns: 200KB }`.
- **Rate limits & why there are no presets**: providers do not publish standard first-byte latencies, so `first_byte` and `throughput` must be measured/edited (Studio defaults `25ms` / `80MB/s` placeholders and `rate_limit: { rps: 5500, burst: 5500 }`). Published per-prefix/bucket reference limits (2026-11-04) are below.

| Provider                 | Published limit                                                                                                           |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| **AWS S3**               | >= `3,500` PUT/COPY/POST/DELETE and `5,500` GET/HEAD req/s per prefix (`85`–`90 MB/s` guideline; returns `503 Slow Down`) |
| **Google Cloud Storage** | ~`5,000` reads and `1,000` writes req/s per bucket initially                                                              |
| **Azure Blob**           | up to `3,000` req/s per block blob (account-bounded throughput)                                                           |

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

Every node with a slot limit defaults to `queue: 100`, and `gateway` defaults to `timeout: 30s` (written into the file only; unset still means unlimited `queue` and no `timeout`). Both `kaanal init` and Studio starters use these same estimates.
