# Nodes reference

Every node kind you can put under `nodes:`, its fields, and what each field does today. Set the kind with `type:`. A field that does not belong to a kind is rejected when the file loads.

Fields marked **active now** change the simulation. Fields marked **not active yet** are accepted and checked (a bad value is still an error) so your file will not need rewriting later, but they have no effect on timing yet.

How a request walks these nodes, and how to change the walk per route, is in the [endpoints reference](endpoints.md).

Durations are `5ms`, `{ p50: 4ms, p99: 20ms }` or `{ mean: 5ms }`, as described in the [user guide](user-guide.md).

## `gateway`

A reverse proxy, API gateway or load balancer. Optional, at most one. With a gateway, every request pays its `proc` first, then runs its auth calls, and it answers static endpoints (an `endpoints:` entry with `override` and no owning service).

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

## Nodes added in Studio

`add_node` (see the [control API](control-api.md#draft)) writes a node that is valid as it stands and has a capacity, so the bottleneck report can rank it. These are **estimates**: typical sizes for a small deployment, meant to be edited.

| Kind           | Written as                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------ |
| `gateway`      | `{ type: gateway, concurrency: 256 }`                                                                        |
| `service`      | `{ type: service, concurrency: 32 }`                                                                         |
| `monolith`     | `{ type: monolith, concurrency: 64 }`                                                                        |
| `database`     | `{ type: database, flavor: postgres, pool: { size: 20 } }` (the flavor fills the read, write and scan costs) |
| `cache`        | `{ type: cache, flavor: redis }` (no capacity; the flavor fills hit ratio and costs)                         |
| `external-api` | `{ type: external-api, pool: { size: 10 } }`                                                                 |
| `auth`         | `{ type: auth, concurrency: 32 }` (needs a signing key, which `kaanal up` and `kaanal studio` provide)       |

## Edges: who may call whom

An edge `{ from: a, to: b, net: 1ms, calls: 2 }` means `a` depends on `b`. `net` is the network latency (default zero). `calls` is how many times `b` is called per request (default 1, at least 1; active now: how many times a chain calls the edge, at most 1 000 when repeated). `prob` (above 0, at most 1, default 1) is the share of requests that take the edge; see [Endpoints](endpoints.md#steps). `bandwidth` (bytes per second, at least 1) limits the link: with a payload size a transfer takes `bytes / bandwidth` on a FIFO link, and without it the link costs nothing and adds no hop. No cycles, no self edges, no duplicate edges.

| From                                | May call                                                           |
| ----------------------------------- | ------------------------------------------------------------------ |
| `gateway`                           | `service`, `monolith`, `auth`                                      |
| `service`, `monolith`               | `service`, `monolith`, `database`, `cache`, `external-api`, `auth` |
| `auth`                              | `service`, `monolith`, `database`, `cache`, `external-api`         |
| `database`, `cache`, `external-api` | nothing                                                            |

An edge that breaks the table is rejected with a message listing what that kind may call.

An edge `gateway -> auth` makes every request pay an auth check; logins run inside it.

## Payload cost

Off by default. `per_kb` on any node (a duration like `proc`, e.g. `per_kb: 50us`) adds that much processing time for every KB of the request and of the response, once per body. `bandwidth` on an edge (bytes per second) makes the link a FIFO queue with one server at that speed. Payload sizes come from the traffic (a stream's `req_bytes` and `resp_bytes` in the simulator; in the live mock, the request's `Content-Length` and the size of the JSON body the route would answer). A route with a collection or a built-in login or JWKS counts its response as 0 bytes. With no sizes, neither field changes anything. In a trace a transfer shows as `db.link=50.0ms` (request) and `db.link-resp=12.0ms` (response), the node being the edge's target. A cache takes `bandwidth` on its edge but no `per_kb` (a load error).

**Network card (`bandwidth` on a node).** Any node except a cache takes `bandwidth:` in bytes per second (at least 1): one number for both directions, or `{ out: 125000, in: 250000 }` (either side may be left out; a missing side is unlimited). It is the node's network card, so it is shared by every wire in that direction: calls to different services from one node share its `out`, and the replies it sends share it too. Each copy has its own send and receive pipe, so more copies mean more bandwidth. A transfer from A to B is pipelined: it lasts `bytes / the narrowest of A's out, the wire's bandwidth and B's in`, and queues FIFO on that narrowest pipe (the wire on a tie, then the sender). Limits of this model: only the narrowest pipe queues the transfer, concurrent transfers queue instead of sharing the rate, and there is no connection set-up time. A load balancer is just a node with a `bandwidth`. In a trace a transfer held up by a card shows as `app.nic-out=20.0ms` or `app.nic-in=20.0ms`, the node being the one whose card it is. Unset means no limit and no hop. The live mock sizes the request from `Content-Length` only (a chunked upload counts 0), counts a HEAD response and a collection or built-in responder as 0 bytes, and sizes the normal success body even when a pin replaces it.

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

| `balance`                 | Picks                                                                         |
| ------------------------- | ----------------------------------------------------------------------------- |
| `round-robin` (default)   | The next live replica in turn                                                 |
| `random`                  | A uniformly random live replica                                               |
| `{ weighted: [3, 1, 1] }` | A random live replica, weighted: one weight per replica, at least one above 0 |

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
