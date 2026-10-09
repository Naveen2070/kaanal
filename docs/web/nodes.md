# Nodes reference

Every node kind you can put under `nodes:`, its fields, and what each field does today. Set the kind with `type:`. A field that does not belong to a kind is rejected when the file loads.

Fields marked **active now** change the simulation. Fields marked **not active yet** are accepted and checked (a bad value is still an error) so your file will not need rewriting later, but they have no effect on timing yet.

> **Latency is optimistic for deeper topologies.** Today a request is charged the entry node's `proc` and the database op costs. A `proc` on a node below the entry (a service called by another service, an external API, an auth node) is accepted but not charged until endpoint chains arrive, so a topology with callees below the entry reports lower latency than the real thing would show.

Durations are `5ms`, `{ p50: 4ms, p99: 20ms }` or `{ mean: 5ms }`, as described in the [user guide](user-guide.md).

## `gateway`

A reverse proxy, API gateway or load balancer. Optional, at most one. With a gateway, every request pays its `proc` first and it answers static `routes`.

```yaml
nginx: { type: gateway, flavor: nginx, proc: 1ms }
```

| Field         | Meaning                                                       | Default   | Status     |
| ------------- | ------------------------------------------------------------- | --------- | ---------- |
| `flavor`      | Free-text label shown in status output (`nginx`, `envoy`)     | none      | active now |
| `proc`        | Processing time per request                                   | zero      | active now |
| `concurrency` | Most requests served at once (live mode warns and ignores it) | unlimited | active now |

## `service`

A backend service. It may carry an OpenAPI 3.0 `spec`. Without a gateway, the routes in its `spec` are served here.

```yaml
app: { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
```

| Field         | Meaning                                                                                 | Default   | Status                                    |
| ------------- | --------------------------------------------------------------------------------------- | --------- | ----------------------------------------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                           | none      | active now                                |
| `proc`        | Processing time per request; charged on the entry node, on others used from A2 (chains) | zero      | active now (entry), used from A2 (chains) |
| `concurrency` | Most requests served at once                                                            | unlimited | active now                                |

## `monolith`

A service that can also be its own database. With `ops` or a `flavor` it has inline read/write/scan costs; with edges to databases it behaves as a service. Both together are allowed.

```yaml
core: { type: monolith, spec: openapi.yaml, proc: 3ms, flavor: mysql, ops: { read: 1ms } }
```

| Field         | Meaning                                                                                 | Default   | Status                                    |
| ------------- | --------------------------------------------------------------------------------------- | --------- | ----------------------------------------- |
| `spec`        | OpenAPI 3.0 file, relative to the config file                                           | none      | active now                                |
| `proc`        | Processing time per request; charged on the entry node, on others used from A2 (chains) | zero      | active now (entry), used from A2 (chains) |
| `ops`         | Inline `read`, `write`, `scan` costs; any you leave out comes from `flavor`             | none      | used once endpoint chains arrive          |
| `flavor`      | Preset that fills `ops` you left out: `postgres`, `mysql`, `mongodb`                    | none      | used once endpoint chains arrive          |
| `concurrency` | Most requests served at once                                                            | unlimited | active now                                |

## `database`

A data store with costs for `read`, `write` and `scan`. Give all three, or a `flavor` to fill the gaps.

```yaml
db: { type: database, flavor: postgres, ops: { write: 9ms } }
```

| Field         | Meaning                                                                                                | Default   | Status                               |
| ------------- | ------------------------------------------------------------------------------------------------------ | --------- | ------------------------------------ |
| `ops`         | `read`, `write`, `scan` costs; required in full unless `flavor` fills them                             | none      | active now                           |
| `flavor`      | Preset for ops you left out: `postgres`, `mysql`, `mongodb`                                            | none      | active now                           |
| `pool`        | Connection pool: `size` (at least 1), `queue` (default 0), `overflow_status` (500 to 504, default 503) | none      | accepted and checked, not active yet |
| `concurrency` | Most requests served at once                                                                           | unlimited | active now                           |

## `cache`

A cache in front of slower nodes, with a hit ratio and the cost of a hit and of a miss. Without a `flavor`, `hit_ratio`, `hit` and `miss` are all required.

```yaml
cache: { type: cache, flavor: redis, hit_ratio: 0.8 }
```

| Field         | Meaning                                                        | Default     | Status                               |
| ------------- | -------------------------------------------------------------- | ----------- | ------------------------------------ |
| `flavor`      | Preset for `hit_ratio`, `hit` and `miss`: `redis`, `memcached` | none        | used once endpoint chains arrive     |
| `hit_ratio`   | Share of lookups that hit, 0 to 1                              | from flavor | used once endpoint chains arrive     |
| `hit`         | Cost of a hit                                                  | from flavor | used once endpoint chains arrive     |
| `miss`        | Cost of finding out it is a miss                               | from flavor | used once endpoint chains arrive     |
| `ttl`         | Entry lifetime                                                 | none        | accepted and checked, not active yet |
| `concurrency` | Most requests served at once                                   | unlimited   | active now                           |

## `external-api`

A third-party HTTP API such as payments or email.

```yaml
pay: { type: external-api, proc: 50ms, error_rate: 0.01 }
```

| Field         | Meaning                                                | Default   | Status                               |
| ------------- | ------------------------------------------------------ | --------- | ------------------------------------ |
| `proc`        | Response time of the API                               | zero      | used from A2 (chains)                |
| `error_rate`  | Baseline share of failing calls, 0 to 1                | none      | accepted and checked, not active yet |
| `rate_limit`  | Token bucket: `rps` (above 0) and `burst` (at least 1) | none      | accepted and checked, not active yet |
| `proxy`       | Real upstream URL (`http://` or `https://`)            | none      | accepted and checked, not active yet |
| `pool`        | Connection pool, as on `database`                      | none      | accepted and checked, not active yet |
| `concurrency` | Most requests served at once                           | unlimited | active now                           |

## `auth`

An identity provider: login and token issue.

```yaml
login: { type: auth, proc: 5ms, token_ttl: 2h, users: [{ username: alice, password: secret, roles: [admin] }] }
```

| Field         | Meaning                                                           | Default   | Status                  |
| ------------- | ----------------------------------------------------------------- | --------- | ----------------------- |
| `proc`        | Cost of one login or token check                                  | zero      | used from A2 (chains)   |
| `users`       | Accounts that can log in (`username` unique, `password`, `roles`) | none      | used from sub-project E |
| `token_ttl`   | Lifetime of issued tokens, at most 1 day                          | 1h        | used from sub-project E |
| `concurrency` | Most requests served at once                                      | unlimited | active now              |

## Edges: who may call whom

An edge `{ from: a, to: b, net: 1ms, calls: 2 }` means `a` depends on `b`. `net` is the network latency (default zero). `calls` is how many times `b` is called per request (default 1, at least 1; used once endpoint chains arrive). No cycles, no self edges, no duplicate edges.

| From                                | May call                                                           |
| ----------------------------------- | ------------------------------------------------------------------ |
| `gateway`                           | `service`, `monolith`, `auth`                                      |
| `service`, `monolith`               | `service`, `monolith`, `database`, `cache`, `external-api`, `auth` |
| `auth`                              | `service`, `monolith`, `database`, `cache`, `external-api`         |
| `database`, `cache`, `external-api` | nothing                                                            |

An edge that breaks the table is rejected with a message listing what that kind may call.

## The gateway is optional

With no gateway, a request enters at the `service` or `monolith` that owns the route in its `spec`. With a gateway, every node that has a `spec` needs an edge from the gateway. Static `routes:` need a gateway.

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
kaanal.yaml:12:5: node 'orders-db': unknown field 'bogus' (expected: concurrency, flavor, ops, pool)
```

Run `kaanal schema` to print a JSON Schema of the whole file for editor autocomplete.
