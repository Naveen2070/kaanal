---
title: "Resources, replicas and presets"
sidebar:
  order: 5
---

## Payload cost

Off by default. Payload sizes come from the traffic profile (`req_bytes` and `resp_bytes`) in the simulator, or from `Content-Length` (chunked uploads are counted as they arrive up to `1 GiB`) and the response body size in the live mock. `collection:` and built-in auth routes count `0` bytes in the simulator (`HEAD` is always `0`).

| Field       | Applies to              | Effect                                                                                                |
| ----------- | ----------------------- | ----------------------------------------------------------------------------------------------------- |
| `per_kb`    | any node except `cache` | Adds processing time per KB of body (`per_kb: 50us`)                                                  |
| `bandwidth` | edges and nodes         | Shares transfer capacity fairly across concurrent transfers (`db.link=50.0ms`, `db.link-resp=12.0ms`) |

`per_kb` and storage `throughput` count a different body depending on the call:

| Call                                               | Body counted  |
| -------------------------------------------------- | ------------- |
| **Writes** (`PUT`, `POST`, `PATCH`, `DELETE`)      | Request body  |
| **Reads** (`GET /{id}`) and **scans** (`GET` list) | Response body |
| Intermediate service hops                          | Both          |

### Per-call sizes (`sends`, `returns`)

By default, the request and response sizes pass down every call. An edge or a `call:` step can override them (`sends: 200KB`, `returns: ...`, max `1 GiB`; step beats edge). Downstream siblings keep the caller's original sizes once the overridden call finishes.

### Headers (`header_bytes`)

HTTP headers dominate small calls on slow links. Top-level `header_bytes:` sets them (`header_bytes: 1KB` or `{ request: 600B, response: 400B }`).

- **Default**: `600 B` request, `400 B` response (`header_bytes: 0` disables them).
- Charged on every wire transfer and in `per_kb` on HTTP nodes (`service`, `monolith`, `gateway`, `auth`, `load-balancer`, `external-api`, `storage` — not database ops).

### Connection set-up (`tls`, `keepalive`)

Calls over an edge with `net` ride a connection:

- **`keepalive`**: `1` in `keepalive` calls (default `1000`, nginx's `keepalive_requests`; `1` = cold connection every call) opens a new connection; the rest reuse a warm one with no set-up cost.
- **Handshake & slow start**: a new connection pays **1 RTT** for TCP (RFC 9293), **+1 RTT** for TLS 1.3 (`tls: 1.3` or `tls: true`) or **+2 RTT** for TLS 1.2 (`tls: 1.2`), plus **TCP slow start** from an initial window of `10 × 1460 B` (RFC 6928) doubling each RTT (RFC 5681) until filling `bandwidth × RTT` (the narrowest of the wire and the two cards; an edge with no `bandwidth` never fills, so a 1 MiB body waits 6 round trips) (logged as `connect` and `connect response`). Edges without `net` pay no set-up; `keepalive: 1000000` effectively disables it.
- **Not modelled**: connection pool exhaustion, idle closes, slow-start-after-idle, session resumption/0-RTT, and HTTP/2–3 multiplexing.

### Streamed responses & uploads (`stream`)

By default responses are buffered (`callee work + wire transfer`). When streamed, transfers overlap the callee's work from its **first byte**, costing `first_byte + size / slowest stage`:

- **Defaults**: `storage` reads (`GET`) stream responses and `storage` writes (`PUT`) stream requests automatically; everything else is buffered.
- **Overrides**: `stream: true` (both), `false` (neither), `response`, or `request` on an edge or `call:` step (step beats edge). Shown as a fork in traces (`first-byte` wait + `link`).
- **Limits**: a callee ending in another call cannot overlap early; chunk-level store-and-forward, client back-pressure, and inter-stage TCP windows are not modelled.

### Packet loss (`loss`)

`loss: 0.01` on an edge drops 1% of packets (default none; edges without `net` pay nothing):

- **Short flows** (`expected losses <= 32`): draws binomial lost segments and charges `1 RTT` when `>= 3` segments follow (fast retransmit, RFC 5681), or `RTO = max(200 ms, 3 × RTT)` otherwise (RFC 6298 / Padhye et al. 1998).
- **Long flows**: paced at the steady-state **PFTK TCP rate** (RFC 5348). Handshake SYN losses wait `1 s`. Logged as `retransmit` / `retransmit response`.
- **Not modelled**: repeated loss on a retransmission, SACK, ECN, and reordering.

### Compression (`compress`)

`compress: 0.3` on an edge shrinks response bodies to `30%` on the wire (gzip/brotli on JSON is typically `0.1`–`0.3`; headers are uncompressed). The node itself still pays `per_kb` and `throughput` on the uncompressed body.

### Units for sizes and rates

- **Byte fields** accept integers (bytes or bytes/s) or unquoted unit strings (`250KB`, `1.5MB`, `2GB`, `512B`; `KiB`/`MiB`/`GiB` aliases; **`1 KB = 1024 B`**).
- **Rates** (`bandwidth`, `bandwidth_cap`) also accept `/s` (`1MB/s`, `250kb/s`); sizes (`req_bytes`, `resp_bytes`) refuse `/s`.
- **Load errors**: a negative number, a missing number (`KB`), `NaN`, an unknown unit (`5XB`) or a value past 2^64 bytes is refused, for example ``kaanal.yaml:6:40: edge #0: field 'bandwidth': bandwidth: unknown unit in `5XB`; use B, KB, MB, GB, optionally /s for a rate``. A fraction rounds to the nearest whole byte. Bounds (bandwidth at least 1 byte a second, sizes at most 1 GiB) apply after the conversion, so `bandwidth: 0KB` fails like `0`.
- **Studio** displays the same value for `250KB` and `256000` and preserves untouched file text on Save.

### Network card (`bandwidth` on a node)

Every node except `cache` accepts `bandwidth:` (`>= 1 B/s`, single rate or `{ out, in }` such as `{ out: 125000, in: 250000 }`; missing side is unlimited).

- **Fair sharing**: all wires leaving/entering a node share that replica's NIC pipe via **max-min fair sharing** (no head-of-line blocking from large uploads; each replica gets its own pipe).
- **Cut-through pipeline**: a transfer takes `bytes / min(sender out, wire, receiver in)` (the narrowest of the three) plus fair-sharing wait (`app.nic-out=20.0ms`, `app.nic-in=20.0ms`).
- **Limits**: rates floor to whole bytes/s; no per-transfer TCP window cap; live mocks do not share pipes across real requests.

## CPUs: `cpu`

```yaml
nodes:
  app: { type: service, concurrency: 200, proc: 10ms, cpu: { cores: 4 } }
```

Off by default. With `cpu: { cores: N }`, a node's **own work** (`proc`, database/monolith ops) uses **processor sharing** (`min(1, N / n)` speed when `n` requests run concurrently; Kleinrock vol. 1 §4.4).

- **`cores`**: `0.001`–`4096` per replica (fractions like `0.5` model container limits). Supported on `gateway`, `service`, `monolith`, `database`, `auth`, and `load-balancer` (refused on `cache`, `external-api`, `storage`).
- **`concurrency` vs `cores`**: slots control admission; cores control execution speed. Downstream waits, wires, and delays consume no CPU; failed requests hold CPU until their active hop ends.
- **Summary & live mocks**: adds `cpu_load`, `cpu_utilization`, `cpu_capacity_per_s`, and `saturates_by` (`slots`, `memory`, or `cpu`). `cpu_load` is the offered load (own work over cores x copies x window); `cpu_utilization` is the CPU really used, at most 1, and any work-conserving server uses `lambda x E[S]` cores on average, so it equals `cpu_load` while the CPUs keep up. `cpu_load >= 1` marks the node saturated (**CPU bound**). Live mocks share the same CPU engine.
- **Accuracy**: single-core matches M/G/1 processor sharing (mean time `E[S] / (1 - rho)` for any service distribution) and `c` cores match Erlang C within ~2%. Context switches, caches, and hyperthreading are not modelled.

## Memory: `memory`

```yaml
nodes:
  app:
    type: service
    proc: 10ms
    memory: { limit: 4GB, per_request: 512MB, base: 1GB }
```

A hard admission limit per replica: holds at most `(limit - base) / per_request` concurrent requests (`6` above; the lower of `concurrency` and memory applies). Without a `concurrency` queue, excess requests are refused immediately with `503` (`saturates_by: memory`).

- **Fields**: `per_request` is required; `limit` may come from `instance:`; `base` defaults to `0` (`B`, `KB`, `MB`, `GB` supported). Supported on the same kinds as `cpu`; `hold: call` keeps memory across downstream calls.
- **Garbage collection (`gc`)**: optional estimate (`memory: { ..., gc: 0.2 }`, try `0.05`–`0.5`) scaling work by `1 + gc × u / (1 - u)` where `u` is live memory share at hop start (`<= 0.99`, Appel/Jones–Hosking–Moss).
- **Accuracy**: without a queue, matches Erlang B (`M/G/c/c`) within 1 percentage point. With `on_limit: kill`, a copy with one request of room, Poisson arrivals and exponential service fails the share `(lambda R + 2) / (lambda (T0 + R))` with `T0 = (2 lambda + mu) / lambda^2`, tested within 2 percentage points. Swapping is not modelled.

### Killing the copy: `on_limit: kill`

```yaml
memory: { limit: 4GB, per_request: 512MB, on_limit: kill, restart: 20s }
```

Instead of refusing (`on_limit: refuse`), `kill` mimics the OOM killer when **memory** is the limiting cap:

- Fails everything running or queued on that replica immediately with **502** (`oom_kills`, cause `oom`, log flag `OM`; retries apply normally).
- Takes that replica offline for `restart` (default `10s`, `<= 1h`), returning **503** before coming back empty.
- `least-conn` and `p2c` skip killed replicas; `round-robin`, `random`, and `hash` do not (pair with `replicas` and `detect` to observe caller impact).

## Cloud instances: `instance`

```yaml
nodes:
  app: { type: service, proc: 10ms, instance: aws/m5.large, memory: { per_request: 256MB } }
```

`instance: provider/type` fills unset `cpu.cores`, `bandwidth`, and `memory.limit` from ~3,300 built-in cloud types (`aws/` ~1,400, `azure/` ~1,300, `gcp/` ~550; case-insensitive), for example `aws/m5.large`, `azure/standard_d4s_v5`, `gcp/n2-standard-8`. Written fields always win. An unknown name is refused with the provider's type count and a few close names.

- **Capacity, not speed**: `proc` stays yours. vCPUs count as full cores (upper bound; burstable `t3`/`e2-medium`/`B` types list full vCPU).
- **Network figures** come from each cloud's own documentation:

| Cloud | Source of the figure                                                                                                                    | Gaps                            |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| AWS   | Stated Gigabit specs                                                                                                                    | `47` older types have none      |
| GCP   | `2 Gbit/s` per vCPU (max `32`, `16` on E2)                                                                                              |                                 |
| Azure | Documented page figures (about 260 sizes of the common families; a constrained-vCPU size such as `E16-8s v6` takes its parent's figure) | `989` of `1,337` sizes have one |

Types without a figure leave `bandwidth` unlimited.

- **Catalogue**: `crates/kaanal-core/data/instances.csv` (refreshed via `scripts/gen-instances.mjs`, served at `GET /__kaanal/instances`).

### Where it runs: `hosting`

```yaml
nodes:
  orders-db: { type: database, flavor: postgres, hosting: managed, engine: postgresql, instance: aws/db.m5.large }
  report:    { type: service, hosting: serverless, proc: 40ms }
```

| `hosting`        | Kinds                                  | `instance:`                                                        | Cost estimate                                        |
| ---------------- | -------------------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------- |
| `self` (default) | any with an instance                   | VM type (`aws/m5.large`) or custom                                 | VM hourly price                                      |
| `managed`        | `database`, `gateway`, `load-balancer` | RDS/Azure SQL class (`aws/db.m5.large`) on DBs; none on gateway/LB | DB class price by `engine`; gateway/LB priced by use |
| `serverless`     | `service`, `monolith`, `database`      | none                                                               | priced by use (Lambda on service/monolith)           |
| `external`       | `database`, `auth`                     | none                                                               | left out ("run by someone else")                     |

- **Managed databases & engines**: the `flavor` maps to these `engine` values:

| `flavor`   | `engine` values                   |
| ---------- | --------------------------------- |
| `postgres` | `postgresql`, `aurora-postgresql` |
| `mysql`    | `mysql`, `aurora-mysql`           |
| `mariadb`  | `mariadb`                         |
| `mongodb`  | `docdb`                           |
| `neo4j`    | `neptune`                         |

Also covered: licence-included RDS editions `sqlserver-standard`, `sqlserver-enterprise`, `sqlserver-web`, `sqlserver-ex` and `oracle-se2`, and Azure SQL (`engine: azure-sql`, compute only): `azure/db.gp_gen5_<n>` General Purpose and `azure/db.bc_gen5_<n>` Business Critical, n = 2 to 80 vCores. `cassandra` and `dynamodb` have no managed class (a `managed` one that names an `aws/db.*` class is refused; size it yourself or use `instance: your-own` with `cost_per_hour`); GCP Cloud SQL is not in the table.

- **`multi_az: true`**: cost-only standby on `managed` non-Aurora DBs (`2×` on Postgres/MySQL/MariaDB; exact AWS Multi-AZ price on Oracle/SQL Server where listed, else `2×`).
- **Managed caches**: `aws/cache.m5.large` etc.; `flavor: valkey` prices at 20% below Redis OSS (Graviton `r8gd` classes are Valkey-only).
- **Per-use billing**: `serverless` service/monolith prices as AWS Lambda (`$0.20/M` + GB-seconds of `proc` at `memory.limit` or `128 MB`); `managed` gateway defaults to API Gateway HTTP API (`$1.00/M` in `us-east-1`), or `api: rest` (`$3.50/M`). Both populate after a run.

### Cost estimate & `instances:`

- **Live card & runs**: sums `copies × hourly on-demand Linux price` (no storage, reserved/spot, or free tiers). Runs also compute **egress** (`sum(rate × resp_bytes)` at the entry node's cloud rate minus free allowance), recorded as `cost.egress` on the run record.
- **Regions**: top-level `region: { aws: eu-west-1, azure: us-east, gcp: europe-west4 }` (defaults `us-east-1`, `us-east`, `us-central1`), served via `GET /__kaanal/prices`.
- **Custom machines (`instances:`)**: define on-prem/custom boxes at the top of `kaanal.yaml` (`cores` required; optional `memory` such as `64GB`, `bandwidth`, `cost_per_hour` in dollars an hour, 0 to 1,000,000, for example `cost_per_hour: 0.40`; names without `/`).

```yaml
instances:
  build-box: { cores: 32, memory: 128GB, bandwidth: 10GB }
  edge-pi:   { cores: 4 }
nodes:
  app: { type: service, instance: build-box, memory: { per_request: 512MB } }
  gw:  { type: gateway, instance: edge-pi }
```

## What a slot covers: `hold`

| `hold`          | Slot held for                               | Fits                                                 |
| --------------- | ------------------------------------------- | ---------------------------------------------------- |
| `own` (default) | Node's own `proc` work only                 | Event loops (Node, Go, async Python)                 |
| `call`          | Admission until all downstream calls finish | Thread-per-request (Tomcat, Gunicorn, Puma, PHP-FPM) |

With `hold: call`, slow dependencies saturate upstream slots and queues (failed/timed-out calls free the caller's slot once its own hop ends; wire response transfer runs after release). Studio's **Thread per request** preset writes `hold: call` and defaults `concurrency: { limit: 200, queue: 100 }` if unset. Per-wire holds and continue-after-failure are not built yet.

## Replicas and balancing

`replicas: N` (`1`–`64`, default `1`) gives each copy its own slots, queue, pool, NIC, CPU, memory, and rate limit. `balance` (on node or overridden on edge) picks a live copy:

| `balance`                 | Behaviour                                                       |
| ------------------------- | --------------------------------------------------------------- |
| `round-robin` (default)   | Next live replica in turn                                       |
| `random`                  | Uniformly random live replica                                   |
| `least-conn`              | Live replica with fewest busy + queued requests at send time    |
| `p2c`                     | Power of two choices: lesser loaded of two random live replicas |
| `{ weighted: [3, 1, 1] }` | Weighted random across replicas (one weight per copy)           |

- **Dynamic balancing (`least-conn`, `p2c`)**: evaluated at hop dispatch in both simulator and live mocks (not valid on a load-balancer's `spread`). Per-replica `down` and latency work; per-replica error rates fall back to node-wide chaos.
- **Per-replica chaos & traces**: `kaanal node down app --replica 1`, `kaanal chaos set app --replica 2 --latency 50ms`; output names a replica `app#1`. Traces report `db#2.read=4.1ms` and `kaanal node list` shows `db x3`. Downed caches fail open as misses; multi-replica total outages fail at call start before the network hop.

## The gateway is optional

Without a gateway, requests enter at the `service` or `monolith` owning the route (`examples/gatewayless`). With a gateway, every `spec:` node needs an edge from it, and static `endpoints:` overrides require a gateway.

## Presets fill what you leave out

`flavor` fills omitted `ops` or cache timings (all estimates for a warm node, network excluded):

| Flavor                      | `read` p50 / p99 | `write` p50 / p99 | `scan` p50 / p99 |
| --------------------------- | ---------------- | ----------------- | ---------------- |
| `postgres`                  | 0.3 ms / 2 ms    | 1 ms / 8 ms       | 3 ms / 25 ms     |
| `mysql`                     | 0.3 ms / 2.5 ms  | 1.2 ms / 10 ms    | 3 ms / 30 ms     |
| `mariadb`                   | 0.3 ms / 2.5 ms  | 1.2 ms / 10 ms    | 3 ms / 30 ms     |
| `sqlserver` _(placeholder)_ | 0.3 ms / 2 ms    | 1 ms / 8 ms       | 3 ms / 25 ms     |
| `oracle` _(placeholder)_    | 0.3 ms / 2 ms    | 1 ms / 8 ms       | 3 ms / 25 ms     |
| `mongodb`                   | 0.4 ms / 3 ms    | 0.8 ms / 6 ms     | 4 ms / 35 ms     |
| `neo4j` _(placeholder)_     | 0.5 ms / 4 ms    | 2 ms / 15 ms      | 6 ms / 50 ms     |
| `cassandra` _(placeholder)_ | 1 ms / 8 ms      | 0.5 ms / 4 ms     | 8 ms / 60 ms     |
| `dynamodb`                  | 3 ms / 9 ms      | 5 ms / 15 ms      | 10 ms / 40 ms    |

| Cache flavor | `hit_ratio` | hit p50 / p99    | miss p50 / p99   |
| ------------ | ----------- | ---------------- | ---------------- |
| `redis`      | 0.9         | 0.2 ms / 1 ms    | 0.2 ms / 1 ms    |
| `memcached`  | 0.9         | 0.15 ms / 0.8 ms | 0.15 ms / 0.8 ms |
| `valkey`     | 0.9         | 0.2 ms / 1 ms    | 0.2 ms / 1 ms    |

## Every error names file, line, column, node and field

```text
kaanal.yaml:12:5: node 'orders-db': unknown field 'bogus' (expected: balance, bandwidth, concurrency, flavor, locale, ops, per_kb, pool, rate_limit, replicas, timeout)
```

Run `kaanal schema` to print the JSON Schema for editor autocomplete.
