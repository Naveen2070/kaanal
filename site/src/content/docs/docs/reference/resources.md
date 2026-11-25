---
title: "Resources, replicas and presets"
sidebar:
  order: 5
---

## Payload cost

Off by default. `per_kb` on any node (a duration like `proc`, e.g. `per_kb: 50us`) adds that much processing time for every KB of the request and of the response, once per body. `bandwidth` on an edge (a rate, see below) gives the link a rate that the transfers on it share fairly. Payload sizes come from the traffic (a stream's `req_bytes` and `resp_bytes` in the simulator; in the live mock, the request's `Content-Length` and the size of the JSON body the route would answer). A route with a collection or a built-in login or JWKS counts its response as 0 bytes. With no sizes, neither field changes anything. In a trace a transfer shows as `db.link=50.0ms` (request) and `db.link-resp=12.0ms` (response), the node being the edge's target. A cache takes `bandwidth` on its edge but no `per_kb` (a load error).

**Sizes per call: `sends` and `returns`.** By default one request size and one response size (the traffic's) are passed down every call. An edge or a `call:` step can say what that call really carries: `sends:` is the request body and `returns:` the response body, written like `req_bytes` (`200KB`, at most 1 GiB). A step beats its edge, and unset passes the incoming size on, so a file without them does not change. A 5 MB upload that the service turns into a 200 KB call to storage is `- { call: files, op: write, sends: 200KB }`; the store is charged for 200 KB, the database call after it still sees the 5 MB, and the sizes go back to the caller's when the call ends.

**Which body a node's size cost counts.** A `per_kb` cost and a storage node's `throughput` count the body that moves, as HTTP and object stores do: a **write** (`PUT`, a database insert, update or delete) counts the request body it receives; a **read** (`GET`) and a **scan** (`LIST`) count the response body it sends. Any other hop (a service doing its own work) counts both. So a download costs the object once, an upload costs it once, and a database write no longer pays for a reply it never sends. This is the rule for the `op` in force: the step's `op:`, else the endpoint's (a `GET` with an id reads, a `GET` without lists, anything else writes).

**Headers.** An HTTP call carries headers as well as a body, and they dominate small calls on slow links. `header_bytes:` at the top of the file sets them: one size for both sides (`header_bytes: 1KB`), or `{ request: 600B, response: 400B }` (a missing side keeps its default). The **default is 600 B on a request and 400 B on a response**, estimates of what an HTTP client sends (cookies and an auth token included) and a server answers, not measurements; `header_bytes: 0` turns them off. They are charged on every wire transfer (so a call with no body still crosses a wire that has a `bandwidth`) and in the `per_kb` cost of an HTTP node (a service, monolith, gateway, auth, load balancer, external API or storage node, not a database op), so a topology with no `bandwidth`, `per_kb` or `throughput` does not change at all. **Upgrading:** a file with a `bandwidth` or a `per_kb` now reports slightly longer times; set `header_bytes: 0` to get the old numbers.

**Connection set-up: `tls` and `keepalive`.** A call over an edge with a `net` rides a connection. One call in `keepalive` (default 1000, nginx's `keepalive_requests`; `1` is a new connection every call) opens a new one; the others reuse a warm connection and pay nothing extra. A new connection pays, before the callee runs, **one round trip for TCP** (RFC 9293), **one more for TLS 1.3** or **two for TLS 1.2** (`tls: 1.3`, `tls: true` or `tls: 1.2`; default none), and then TCP **slow start**: the first flight is 10 segments of 1460 B (RFC 6928) and the window doubles every round trip (RFC 5681), so a body that does not fit waits one round trip per doubling until the window fills the pipe (the narrowest of the wire and the two cards, `bandwidth x round trip`; an edge with no `bandwidth` never fills, so a 1 MiB body waits 6 round trips). A round trip is the call's own `net` draw, jitter and chaos included, so **an edge with no `net` pays no set-up**. Both directions start cold on a new connection. Not modelled: a pool that runs out under load, idle-timeout closes, slow-start after idle, session resumption and 0-RTT, HTTP/2 and HTTP/3 multiplexing, packet loss (step 6d). The set-up shows in the log as `connect` and `connect response` hops. This is on by default, so about 1 call in 1000 on an edge with a latency now takes a round trip more; `keepalive: 1000000` all but turns it off.

**Streamed responses: `stream`.** By default a response is buffered: the callee finishes all its work, then the bytes cross the wire, so a 1 MiB read from a store costs `first_byte + size / throughput` and then `size / wire`, added. A **streamed** response crosses the wire while the callee is still producing it, so the call takes `first byte + size / the slowest stage` (the store, a node's card or the wire): the pipeline limit, cut-through as chunks get small. The transfer starts at the callee's **first byte** (the end of its last own hop less that hop's size-dependent part: a store's `first_byte`, a database op's time, a service's `proc`) and shares the wire fairly with other transfers while it is on it. **A read of a `storage` node streams its response by default**, as S3, GCS and Azure send a GET as it is read; everything else is buffered. `stream: true` (both directions), `stream: false` (neither), `stream: response` or `stream: request` on an edge or a `call:` step changes that (a step beats its edge). A callee whose sequence ends in another call cannot send early, so it stays as slow as buffered; a call with no response bytes or no pipe with a rate has nothing to overlap, so it is unchanged; a failing callee sends nothing. In the trace a streamed call is a fork: the callee's hops beside a `first-byte` wait and the response `link`. **An upload streams too:** a **write** to a `storage` node streams its request by default, as a store takes a PUT as the bytes arrive: the request crossing the wire and the store's work run side by side (a fork in the trace), so a 1 MiB PUT over a 1 MiB/s wire to a store that needs 30 ms takes the wire's 1 s, not 1.03 s. `stream: request` does the same for any callee (a service that consumes an upload as it arrives). The response, if it also streams, starts from the callee's first byte as before; a large response to a streamed upload starts when the callee ends, not when the upload does. Not modelled: chunk-level store-and-forward for small bodies, a client that reads slowly (back-pressure), TCP window limits between the stages.

**Packet loss: `loss`.** `loss: 0.01` on an edge says one packet in a hundred is lost; the default is none, so nothing moves until you write it. A lost packet costs TCP a recovery, and the model takes the published one: a body of up to 32 expected losses (`segments x loss`) draws its lost segments (binomial) and pays, for each, **one round trip** when three or more segments follow it (fast retransmit, RFC 5681) or the **retransmission timeout** when they do not (the tail of the body, or a body of a few segments; `max(200 ms, 3 x round trip)`, RFC 6298; the chance is `min(1, 3 / flight)`, Padhye et al. 1998). A longer flow runs at the **steady TCP rate of RFC 5348** (the PFTK equation) and the call pays the time that takes over the lossless time. A new connection's handshake packets are lost the same way, and a lost SYN waits **1 s** (RFC 6298). The cost is in round trips, so **an edge with no `net` pays nothing**. It shows in the log as `retransmit` and `retransmit response` hops. Not modelled: a retransmission lost again, the window halving after a recovered loss inside a short flow, SACK, ECN, reordering.

**Compression: `compress`.** `compress: 0.3` on an edge sends 30% of a response's bytes over the wire (gzip or brotli on JSON is commonly 0.1 to 0.3; already compressed media is about 1). The wire, slow start and loss see the smaller body; the component still handles the whole one, so a store's `throughput` and a node's `per_kb` are unchanged (compressing costs the compressing node's own `per_kb`). Only a response is compressed, and headers are not. Default none; there is no universal ratio, so the number is yours.

**Units for sizes and rates.** Every byte field takes a whole number (bytes, or bytes per second for a rate: unchanged from before) or text with a unit: `250KB`, `250 KB`, `1.5MB`, `2GB`, `512B`. Units are `B`, `KB`, `MB`, `GB` in any case, with `KiB`, `MiB`, `GiB` as the same thing, and **KB = 1024 bytes** (the same as Studio's unit picker, so a number means the same in the file and on the page). A fraction rounds to the nearest whole byte. A rate (node and edge `bandwidth`, the `bandwidth_cap` of a scenario or the control API) may also end in `/s`: `1MB/s`, `250kb/s`; a size (`req_bytes`, `resp_bytes` in [traffic](/docs/guides/simulate/traffic/)) refuses `/s`. A negative number, a missing number (`KB`), `NaN`, an unknown unit (`5XB`) or a value past 2^64 bytes is a load error such as ``kaanal.yaml:6:40: edge #0: field 'bandwidth': bandwidth: unknown unit in `5XB`; use B, KB, MB, GB, optionally /s for a rate``. The bounds (bandwidth at least 1 byte a second, sizes at most 1 GiB) apply after the conversion, so `bandwidth: 0KB` fails like `0`. No quotes are needed (`bandwidth: 250KB` and `bandwidth: 1MB/s` are plain YAML); in the control API's JSON the value is a string. Studio shows the same number for `250KB` and `256000` and keeps the text when it saves a file it did not change; an edit to the field writes whole bytes.

**Network card (`bandwidth` on a node).** Any node except a cache takes `bandwidth:` as a rate (at least 1 byte per second): one number for both directions, or `{ out: 125000, in: 250000 }` (either side may be left out; a missing side is unlimited). It is the node's network card, so it is shared by every wire in that direction: calls to different services from one node share its `out`, and the replies it sends share it too. Each copy has its own send and receive pipe, so more copies mean more bandwidth. A transfer from A to B is pipelined: it lasts `bytes / the narrowest of A's out, the wire's bandwidth and B's in`, when it is alone. Pipes are **shared fairly** (max-min, as TCP converges to): every pipe splits its rate equally between the transfers crossing it, and a transfer held back by a slower pipe leaves its spare share to the others, so a big upload no longer blocks a small call and a fan-out through one card finishes together. The time a transfer loses to sharing is its wait in the trace and the Log. Limits of this model: a share is floored to whole bytes per second (under 1 B/s of error), a transfer has no cap of its own (a TCP window is planned), there is no connection set-up time, and the live mock does not share a pipe between real requests. A load balancer is just a node with a `bandwidth`. In a trace a transfer held up by a card shows as `app.nic-out=20.0ms` or `app.nic-in=20.0ms` (its time alone, the wait showing what sharing added), the node being the one whose card it is. Unset means no limit and no hop. The live mock sizes the request from `Content-Length`, or, when there is none (a chunked upload) and the route does not read the body itself, from the bytes the client really sends (counted as they arrive, up to 1 GiB, and thrown away); a login or a collection route that reads its own body counts 0 without a `Content-Length`. It counts a HEAD response and a collection or built-in responder as 0 bytes, and sizes the normal success body even when a pin replaces it.

## CPUs: `cpu`

```yaml
nodes:
  app: { type: service, concurrency: 200, proc: 10ms, cpu: { cores: 4 } }
```

Off unless written: with no `cpu`, every request takes its drawn time however many run at once, so a thread-per-request server on 4 CPUs with 200 threads looks healthier than it is. With `cpu: { cores: N }` a node's **own work** (its `proc`, and a database's or monolith's operations) is shared among the requests running it: with `n` of them at once each runs at `min(1, N / n)` of its speed, so ten 10 ms requests on 2 cores each take about 50 ms. That is processor sharing, the standard model of a CPU time-sliced between threads (Kleinrock, _Queueing Systems_ vol. 1, section 4.4). The extra time shows as the hop's wait in the Log and in `mean_wait_ms`.

- `cores` is a number from `0.001` to `4096`; a fraction such as `0.5` is a container's CPU limit. Each copy (`replicas`) has its own cores.
- `concurrency` still decides how many requests are admitted; `cores` only decides how fast the admitted ones run. They are separate on purpose: slots are threads, cores are CPUs.
- Taken by `gateway`, `service`, `monolith`, `database`, `auth` and `load-balancer`. A cache, an external API and a storage node have no `cpu` (their time is a given figure, not your CPUs), and the file is refused at that key if you write one.
- **Not stretched:** time waiting on what the node calls (including a `hold: call` slot), the wire, and wait steps use no CPU and run at their drawn time. A request that fails keeps its CPU share until its running hop ends, as it keeps its slot.
- The summary adds `cpu_load` (own work over cores x copies x window; 1 or more means the CPUs cannot keep up), `cpu_utilization` (the CPU the work **really used** in the window over the same, at most 1 (the live ticks carry it too, over each tick's span): a measured figure, where `cpu_load` is the offered one; any work-conserving server uses `lambda x E[S]` cores on average, so it equals `cpu_load` while the CPUs keep up and stays at 1 past that), `cpu_capacity_per_s` and `saturates_by` (`slots`, `memory` or `cpu`, whichever sets `saturates_at_req_s`). A node with `cpu_load` of 1 or more is saturated even with no slot limit. Results labels such a row **CPU bound**.
- The live mock shares the cores too: every real request runs through the same replay engine as a simulation, so concurrent requests slow each other down exactly as in a run.
- Accuracy: a single core matches M/G/1 processor sharing (mean time `E[S] / (1 - rho)` for any service distribution) and `c` cores with exponential service match Erlang C, both tested to within about 2%. Context switching, cache effects and hyperthreads are not modelled: a vCPU counts as a full core, so for a cloud instance use its vCPU count as an upper bound (or let an `instance:` do it, below). Disk presets are planned, not built.

## Memory: `memory`

```yaml
nodes:
  app:
    type: service
    proc: 10ms
    memory: { limit: 4GB, per_request: 512MB, base: 1GB }
```

A **hard limit**, off unless written. Each request being served holds `per_request` bytes on top of the `base` the process holds idle, so a copy holds at most `(limit - base) / per_request` requests at once: here 6. That is a slot count, the same as `concurrency`, and the **lower** of the two applies. A request that finds them taken waits in the node's `concurrency` queue, or, when the node has no `concurrency` of its own, is **refused at once with 503** (memory does not queue). The Results row says **memory** in `saturates_by` when the memory is what holds the copy to that many.

- Sizes take units above 1 GiB (`B`, `KB`, `MB`, `GB`; 1 KB = 1024 bytes). `per_request` is required; `limit` may come from an `instance:`; `base` defaults to 0. A limit that cannot hold one request is refused at load.
- Taken by the same kinds as `cpu`. Each copy has its own memory.
- A `hold: call` slot keeps its memory while the node waits on what it calls, as a thread-per-request server does.
- **Not modelled:** swapping, and the pauses of any one collector (below, a single labelled estimate stands in). What the kernel does at the limit, killing the process, is `on_limit: kill`.
- **Garbage-collection cost: `gc`** (off unless set, an **estimate**). `memory: { ..., gc: 0.2 }` makes the node's own work take `1 + gc x u / (1 - u)` times its drawn time, `u` being the share of the memory that is live when the work starts (`base` plus `per_request` for each request being served, over `limit`; capped at 0.99). The shape is published: for live data `L` in a heap `H` a collector does about `L / (H - L)` work per byte allocated (Appel 1987; Jones, Hosking and Moss, _The Garbage Collection Handbook_, 2011). The **weight `gc`** (the cost of collecting a byte against the work of using it) depends on your runtime and allocation rate, so you set it: `0.05` to `0.5` is a range to try, `0` or absent is none. It uses the load when the work starts and is not updated while it runs. Tested against the formula within 0.5%.
- Accuracy: with no queue the node is an `M/G/c/c` loss system, so the share refused is **Erlang B** for any service distribution, tested to within 1 percentage point.

### Killing the copy: `on_limit: kill`

```yaml
memory: { limit: 4GB, per_request: 512MB, on_limit: kill, restart: 20s }
```

By default a request the memory cannot hold is turned away (`on_limit: refuse`). With `kill` it does what the kernel's OOM killer does: it **kills the whole copy**.

- Everything the copy is serving or has queued fails at that instant with a **502**, and counts as one kill (`oom_kills` in the summary, cause `oom`, flag `OM` in the log). A retry policy retries those calls like any other failure.
- The copy is **down for `restart`** (default `10s`, above 0, at most `1h`): every call that reaches it is refused with **503**. Then it is back, empty.
- The other copies of the node carry on. **`least-conn` and `p2c` skip a killed copy** (they choose when the call is sent, as a balancer with a health check would). **`round-robin`, `random` and `hash` do not**: they choose when the request starts, before the engine runs, and cannot know the copy died, so some calls still go to the dead one and are refused (503); use `replicas` and the balancer's `detect` to see what a caller feels.
- It applies only when the **memory** is what limits the node's slots (a `concurrency` lower than what the memory holds refuses as before). `restart` without `on_limit: kill` is refused at load.
- **The copy is emptied at the kill**, as a real process's memory is: its slots are free, its queue gone, its `hold: call` slots closed and what ran on its CPUs stopped, so a request longer than `restart` leaves nothing behind.
- Accuracy: a copy with one request of room, Poisson arrivals and exponential service fails the share a renewal argument gives, `(lambda R + 2) / (lambda (T0 + R))` with `T0 = (2 lambda + mu) / lambda^2`, tested within 2 percentage points.
- A kill during a live run takes effect for new requests at once.

## Cloud instances: `instance`

```yaml
nodes:
  app: { type: service, proc: 10ms, instance: aws/m5.large, memory: { per_request: 256MB } }
```

`instance: provider/type` fills, from an embedded table, the `cpu.cores`, the network card (`bandwidth`, both directions) and the `memory.limit` you **leave out**. A value you write always wins. The table lists **every** type of the three big clouds, about 3,300 (`aws/` 1,400, `azure/` 1,300, `gcp/` 550), as `aws/m5.large`, `azure/standard_d4s_v5`, `gcp/n2-standard-8`; names are not case sensitive. An unknown name is refused with how many types that provider has and a few that start like it.

- **A preset is capacity, not speed.** Per-core speed differs between families, so your `proc` stays your own figure.
- **Estimates.** vCPU and memory are the providers' own figures. A vCPU is a hardware thread but counts as a full core here, so every figure is an upper bound. **Network:** AWS types carry their stated "Gigabit" figure (older types that say only Low, Moderate or High have none); GCP types use the documented default egress cap of 2 Gbit/s per vCPU, at most 32 (16 for the E2 family); for Azure, the figure Microsoft states on each size's own page (about 260 sizes of the common families, kept by hand in `scripts/azure-network.csv`, and a constrained-vCPU size such as `E16-8s v6` takes its parent's figure); the other Azure sizes have none. **A type with no network figure leaves the card unlimited**, which is what an absent `bandwidth` means.
- Burstable types (`t3`, `e2-medium`, the Azure `B` series) run above their baseline only on CPU credits, which are not modelled: they are listed at their full vCPU count.
- An instance alone sets no memory model: add `memory: { per_request }` to use its memory as the limit.
- The table is `crates/kaanal-core/data/instances.csv`, generated by `scripts/gen-instances.mjs` from the public catalogues at instances.vantage.sh (a mirror of each provider's pages); run it again to refresh and commit the diff. The server serves it at `GET /__kaanal/instances`, so Studio and the loader read one list.
- Studio: under a component's CPUs, pick a provider and search its types; the line below the box says what it fills.

> **Azure network figures are partial.** About 989 of the 1,337 Azure sizes carry the bandwidth Microsoft states on the size's page (`scripts/read-azure-network.mjs` reads every series page of its size docs: only a plain whole number is taken, so a lower bound like "16,000+" is left out); an Azure size without one fills cores and memory but leaves the network card **unlimited**, which can make a wire look faster than the VM really is. Set `bandwidth:` on the node yourself if it matters. AWS has no figure for 47 older types and GCP's figures are the documented per-vCPU estimate, so the same advice applies there. The network figure belongs to the type, not the region: a type has the same card wherever it is offered.

### Where it runs: `hosting`

```yaml
nodes:
  orders-db: { type: database, flavor: postgres, hosting: managed, engine: postgresql, instance: aws/db.m5.large }
  report:    { type: service, hosting: serverless, proc: 40ms }
```

A VM type only fits a machine you run. `hosting:` says what the component is, and so which `instance:` it may name and how the estimate prices it. Leave it out for `self`.

| `hosting`        | Kinds that take it               | `instance:`                                                                                                               | The cost estimate                                                                                         |
| ---------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `self` (default) | every kind with an instance      | a VM type (`aws/m5.large`) or your own                                                                                    | the VM price                                                                                              |
| `managed`        | database; gateway; load balancer | a database takes a **managed class** (`aws/db.m5.large`, from AWS RDS) or your own; a gateway or load balancer takes none | a database class is priced per `engine`; a managed gateway or load balancer is left out ("priced by use") |
| `serverless`     | service; monolith; database      | none                                                                                                                      | left out ("priced per use")                                                                               |
| `external`       | database; auth                   | none                                                                                                                      | left out ("run by someone else")                                                                          |

- **A managed class fills the same things a VM type does:** `cpu.cores`, the network card and the `memory.limit`, from the class's own figures (`db.m5.large` is 2 vCPU and 8 GiB). Its **price depends on the engine** (`engine:` is `postgresql` (default), `mysql`, `mariadb`, `aurora-postgresql`, `aurora-mysql`, or for `flavor: sqlserver` and `oracle` the licence-included editions `sqlserver-standard`, `sqlserver-enterprise`, `sqlserver-web`, `sqlserver-ex`, `oracle-se2`, and `azure-sql`), and on the region you pick at the globe: db.m5.large is $0.178 an hour for PostgreSQL in us-east-1 and $0.197 in eu-west-1 (list prices).
- **Multi-AZ:** `multi_az: true` on a managed database adds a standby in a second zone. On PostgreSQL, MySQL and MariaDB it **doubles the price** (AWS bills the standby as a second instance: exactly 2.00 times the single-AZ price in the price catalogue). For the licence-included Oracle and SQL Server editions the standby has its own price, 1.23 to 2.59 times the instance (it is 2.00 in only 3,640 of 6,181 region and class pairs), so the estimate reads **AWS's Multi-AZ price for the class and region** (RULES 133); only a class with no Multi-AZ row, and SQL Server Express which has none, falls back to twice the single-AZ price, and the node's note says so. It is cost only: the failover is not modelled. It goes with `hosting: managed`, and not with Aurora, which keeps its copies with `replicas` and bills each (both refused at load). In Studio it is a checkbox under the Hosting box.
- **A cache** takes `self` (Redis, Valkey or Memcached on a VM you run), `managed` (ElastiCache or Memorystore: a class like `aws/cache.m5.large`) or `serverless`. Its instance is a **price and a size note only**: a cache's hits and misses are fixed times, so its cores and memory do not change them (the model has no CPU sharing or memory limit for a cache yet). cache.m5.large is $0.156 an hour in us-east-1 and $0.172 in eu-west-1 (Redis OSS list price, Memcached is the same). **`flavor: valkey` is priced as ElastiCache for Valkey, which AWS lists 20% below Redis OSS** ($0.1248 an hour for cache.m5.large in us-east-1): choose the engine in the Hosting box of a managed cache. The newest Graviton classes (`r8gd` and others) are sold with Valkey only, so they are priced for the `valkey` flavor alone. Valkey takes the Redis preset below: no published figure separates them.
- **Billed by use (RULES 131).** A `serverless` service or monolith is priced as **AWS Lambda** from a run's visits: $0.20 per million requests plus $0.0000166667 per GB-second of its mean `proc` at its `memory:` limit (Lambda's 128 MB when none), us-east-1 and other regions from AWS's list; time it spends waiting on what it calls is not counted, so a Lambda that calls others costs more. A `managed` gateway is priced as an **API Gateway HTTP API** by default, $1.00 per million requests in us-east-1 (AWS steers every new build without API keys, usage plans, WAF or a response cache to it); write `api: rest` for a **REST API**, $3.50. The node's note says which was assumed, and `api:` goes only with a gateway that has `hosting: managed` (refused at the field otherwise). In Studio it is the API type select under the Hosting box. Both need a run (or live) rate; before one the card and the CLI say "priced by use, after a run". DynamoDB on demand, ALB and Aurora Serverless are not priced yet.
- **A gateway or load balancer is `self` when you run nginx, Envoy or HAProxy on a VM, in any cloud** (the VM is the instance); `managed` is the provider's (API Gateway, ALB), which has no size to pick and is left out of the estimate. An external API and a storage node have no hosting choice: the first is someone else's by definition, the second is an object store.
- **Which databases have a managed class:** the engine follows the flavor. `postgres` takes `postgresql` or `aurora-postgresql`, `mysql` takes `mysql` or `aurora-mysql`, `mariadb` takes `mariadb`. `mongodb` takes `docdb` (Amazon DocumentDB) and `neo4j` takes `neptune` (Amazon Neptune): their own price lists, from AWS's own price files (db.r5.large is $0.277 an hour on DocumentDB and $0.348 on Neptune in us-east-1), on the classes the table already has. **`cassandra` and `dynamodb` have no managed class** (Keyspaces and DynamoDB bill per use, not by class; Atlas and AuraDB publish no class prices): a `managed` one of those is refused if it names an `aws/db.*` class, and you size it yourself (`cpu`, `memory`) or use `instance: your-own` with `cost_per_hour`. Self-hosted, every flavor is priced as the VM it runs on.
- **Licences and Azure SQL (`flavor: sqlserver` or `oracle`).** The editions are read from AWS's own RDS price files: `oracle-se2`, `sqlserver-standard`, `sqlserver-enterprise`, `sqlserver-web` and `sqlserver-ex` are **licence included** (the licence is inside the hourly price; db.m5.large is $0.977 for SQL Server Standard and $0.438 for Oracle Standard Two in us-east-1, against $0.178 for PostgreSQL). Bring-your-own-licence prices are the plain instance price and are not offered, and Oracle Enterprise is left out (its list has two prices under two product codes). **Azure SQL Database** (`engine: azure-sql`, classes `azure/db.gp_gen5_<n>` General Purpose and `azure/db.bc_gen5_<n>` Business Critical, n = 2 to 80 vCores, 5.1875 GiB each) is the retail API's **compute meter only**: no storage, backup or licence meter. An `azure/` class is always priced as Azure SQL. Cloud SQL (needs a billing API key) is not offered.
- **Azure SQL and AWS RDS are the managed classes in the table** (single-AZ, on-demand). Google Cloud publishes none we can read, so for it set `cores`, `memory` and `bandwidth` yourself. Storage and I/O charges are not in it.
- **`serverless`, `external` and a managed gateway or load balancer take no `instance:`** (the file is refused with the reason). `cpu`, `memory` and `bandwidth` stay yours to write if you model them.
- **The loader refuses a mismatch with the path**: a managed class on a `self` node, a VM type on a managed database, `engine:` without a managed database, or a hosting the kind does not have.
- Studio: the **Hosting** box sits at the top of a component's CPUs group. Changing it drops an `instance:` that no longer fits. A managed database lists only the AWS database classes the region offers.

### What it costs: the cost estimate

Studio shows an **estimate of what the system costs** in a small card under the node picker (the dollar figure on the left). It adds up, for every node that names an instance, the **copies times the price of one hour**, from the on-demand Linux list prices of the table. Click the card for the details: the price of each node, the totals for an hour, a day and **the number of days you type**, and the cost **per 1,000 requests** at the offered rate (the live rate, else the traffic profile's).

- **Egress is in a run's cost (RULES 131), not in the live card.** What a run sends out to the internet (each stream's rate times its `resp_bytes`) is priced at the first-tier internet-egress price of the cloud of the stream's **entry node** (AWS $0.09, Azure $0.087, Google $0.12 a GB at the headline, regional where the cloud lists one), less the free allowance (100 GB a month on AWS and Azure); an entry node that names no cloud is priced at AWS's rate and the run record says so. It is on the run record's `cost.egress` and under the CLI's cost line.
- **It is an estimate, not a quote.** On-demand Linux list prices of one region, instances only: **no** storage, managed-service or (in the live card) egress fees (those are most of a real bill), no reserved, savings-plan or spot prices, no free tiers. Burstable types are priced at their normal rate.
- **A node with no instance, or a type with no price in the region, is left out**, and the card says how many.
- **Your own machine has a price when you give one**: `instances: { rack: { cores: 8, cost_per_hour: 0.40 } }`, dollars an hour (0 to 1,000,000).
- **Region.** Every cloud region has its own price list (`crates/kaanal-core/data/prices/<provider>/<region>.csv`, about 220 regions). The **globe at the top right** picks the region of AWS, Azure and Google Cloud and **writes it into the file** as a top-level `region: { aws: eu-west-1, azure: us-east, gcp: europe-west4 }`, so the project carries it (a provider the file leaves out uses what your browser picked, else its default; the file wins; a name that is not a region of that cloud is refused with the path); the cost estimate and the instance picker both follow it, and the picker lists **only the types the region offers** (a type with a price there is offered there). Defaults: AWS `us-east-1`, Azure `us-east`, GCP `us-central1`.
- The prices come from the same catalogues as the table, in whole micro-dollars so the sums are exact; run `scripts/gen-instances.mjs` to refresh them. The server serves them at `GET /__kaanal/prices` and `GET /__kaanal/prices/{provider}/{region}`.

### Your own machines: `instances:`

A self-hosted server, an on-premises box or another provider's size is not in the table. Describe it once at the top of the file and use it by name:

```yaml
instances:
  build-box: { cores: 32, memory: 128GB, bandwidth: 10GB }
  edge-pi: { cores: 4 }
nodes:
  app: { type: service, instance: build-box, memory: { per_request: 512MB } }
  gw: { type: gateway, instance: edge-pi }
```

- `cores` is required (0.001 to 4096). `memory` is a size (`64GB`); `bandwidth` is a rate for both directions or `{ out, in }`, like a node's; `cost_per_hour` is dollars an hour for the cost estimate (absent: no price). Leave out what the machine does not give: no `memory` means no limit from the instance, no `bandwidth` means an unlimited card.
- A node that names it takes what it leaves out, exactly as with a table type, and what the node writes wins. Several nodes can share one.
- Names have letters, digits, `.`, `_` and `-` (up to 64), **no `/`**: that is how the cloud table is written, so the two never collide. An unknown name is refused, listing yours.
- In Studio choose **Your own** under Instance type: pick one of yours, edit its cores, memory and network, add a new one (a name and its cores), or delete one no node uses.

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
- Studio: the **Server style** choice on the Settings tab, for any component that calls others: **Event loop (default)** writes nothing (`hold: own`); **Thread per request** writes `hold: call` and, on a component with no slot limit yet, also `concurrency: { limit: 200, queue: 100 }` (Tomcat's defaults; change them under Capacity). Choosing the event loop again removes `hold` and leaves the slots as they are.

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

`least-conn` and `p2c` read the live queues, so the **simulator** (Studio runs, `kaanal-sim`) chooses at the moment each call is sent, as a real balancer does; the **live mock** does the same: every real request runs on the same engine, which chooses the copy by its live queues when the call is sent (a node with no limit is gated too in the live engine, so its copies are told apart by the requests they hold). Two cautions: the simulator reads exact queue lengths, so it is a little kinder than a real balancer with delayed counts; and chaos aimed at one replica works with them for `down` and for added latency, but an error rate set on one replica is not applied (the node-wide chaos is). A balancer's `spread` cannot use them (a load error says so): put `least-conn` or `p2c` on the `balance` of the components behind the balancer.

A node sets `balance` for traffic entering it, and an edge may override it for calls over that edge: `{ from: app, to: db, balance: random }`. A weighted list must have exactly one weight per replica of the callee. Without `replicas`, `balance` does nothing. Database replicas all take every request: there is no read/write split yet, so a write is not sent to a primary only.

Chaos can target one replica: `kaanal node down app --replica 1`, `kaanal chaos set app --replica 2 --latency 50ms`, `kaanal chaos clear app --replica 2` (scenario entries and the API take `replica` next to `node`). A downed replica is skipped by the balancer; the other copies keep serving, and a node with every replica down answers 503. Output names a replica `app#1`.

A request's trace names the copy that served each hop (`db#2.read=4.1ms`, and `db#2.read` in `kaanal logs` waterfalls), the logged hop carries `replica`, `GET /__kaanal/topology` lists `replicas` on a replicated node, and `kaanal node list` shows `db x3`. Chaos latency is attributed to the replica it hit.

Two details. A cache never fails a request, so a cache with every replica down is a miss, as a down cache already was. And when a node is down, a call to it with more than one replica fails at the call start (no network hop, no timeout span), while a call to a one-replica node fails at the callee's own hop, after the network hop: the same 503, a different trace.

## The gateway is optional

With no gateway, a request enters at the `service` or `monolith` that owns the route in its `spec`. With a gateway, every node that has a `spec` needs an edge from the gateway. Static endpoints (an `endpoints:` entry with `override` and no owning service) need a gateway.

See `examples/gatewayless` in the repository for a complete gateway-less file.

## Presets fill what you leave out

A `flavor` fills the values you do not write; anything you write wins. **All numbers below are estimates**: typical single-node figures for a warm instance with small rows, network excluded (edges carry `net`).

| Flavor      | read (PK lookup) p50 / p99 | write (single-row commit) p50 / p99 | scan (indexed list, ~100 rows) p50 / p99 |
| ----------- | -------------------------- | ----------------------------------- | ---------------------------------------- |
| `postgres`  | 0.3 ms / 2 ms              | 1 ms / 8 ms                         | 3 ms / 25 ms                             |
| `mysql`     | 0.3 ms / 2.5 ms            | 1.2 ms / 10 ms                      | 3 ms / 30 ms                             |
| `sqlserver` | 0.3 ms / 2 ms              | 1 ms / 8 ms                         | 3 ms / 25 ms                             |
| `oracle`    | 0.3 ms / 2 ms              | 1 ms / 8 ms                         | 3 ms / 25 ms                             |
| `mongodb`   | 0.4 ms / 3 ms              | 0.8 ms / 6 ms                       | 4 ms / 35 ms                             |
| `mariadb`   | 0.3 ms / 2.5 ms            | 1.2 ms / 10 ms                      | 3 ms / 30 ms                             |
| `neo4j`     | 0.5 ms / 4 ms              | 2 ms / 15 ms                        | 6 ms / 50 ms                             |
| `cassandra` | 1 ms / 8 ms                | 0.5 ms / 4 ms                       | 8 ms / 60 ms                             |
| `dynamodb`  | 3 ms / 9 ms                | 5 ms / 15 ms                        | 10 ms / 40 ms                            |

`mariadb` uses the `mysql` figures (InnoDB underneath). `sqlserver` and `oracle` are **placeholders** with the `postgres` figures: no published figure separates them, so edit them. `neo4j` (a node lookup with a hop, a commit, a traversal of about 100 nodes) and `cassandra` (a cheap log-structured write, a read that may touch several SSTables, a partition slice) are **placeholders from typical ranges, not published figures**: no vendor states them, so measure yours and write `read`, `write` and `scan`. `dynamodb` sits inside AWS's "single-digit millisecond" statement. Not preset yet: SQL Server, Oracle, CockroachDB, Elasticsearch.

| Flavor      | `hit_ratio` | hit p50 / p99    | miss p50 / p99   |
| ----------- | ----------- | ---------------- | ---------------- |
| `redis`     | 0.9         | 0.2 ms / 1 ms    | 0.2 ms / 1 ms    |
| `memcached` | 0.9         | 0.15 ms / 0.8 ms | 0.15 ms / 0.8 ms |
| `valkey`    | 0.9         | 0.2 ms / 1 ms    | 0.2 ms / 1 ms    |

## Every error names file, line, column, node and field

```text
kaanal.yaml:12:5: node 'orders-db': unknown field 'bogus' (expected: balance, bandwidth, concurrency, flavor, locale, ops, per_kb, pool, rate_limit, replicas, timeout)
```

Run `kaanal schema` to print a JSON Schema of the whole file for editor autocomplete.
