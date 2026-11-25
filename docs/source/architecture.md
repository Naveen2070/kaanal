# Architecture

This document provides a comprehensive architectural breakdown of Kaanal (கானல்): its design principles, crate boundaries, execution engine, data synthesis model, control plane, security guarantees, and concurrency model.

---

## 1. Design Philosophy & Architectural Invariants

Kaanal simulates a multi-tier microservice backend using a single binary, a single configuration file (`kaanal.yaml`), and OpenAPI 3.0 specifications.

### Core Tenets

1. **One Binary, One Port**: Mock routes and the `/__kaanal/*` control plane share a single TCP port. No secondary listener or out-of-band proxy is required.
2. **Behavioral Modeling, Not Protocol Emulation**: Databases and caches are virtual nodes modeled by their operation latencies and failure probabilities. Kaanal does not execute SQL or emulate wire protocols (rule R2.8).
3. **One Cost Model, Two Drivers**: Latency calculation, distribution sampling, and failure decisions exist solely in `kaanal-core`. One replay engine in `kaanal-core` runs the sampled plans against slots, queues and rate limits; the live server drives it on a real-time clock (one actor task) and `kaanal-sim` on a virtual clock, with no change to the cost model (rules R2.5, R2.10).
4. **Pure Core Engine**: `kaanal-core` contains zero asynchronous code, zero IO, zero threads, and zero network calls. It computes outcomes deterministically given its inputs (rule R2.3).
5. **Absolute Determinism**: For a fixed `seed` in `kaanal.yaml`, identical request sequences produce identical timings, failure distributions, and response payloads.
6. **Thin Interfaces**: Every user interface (CLI, Web UI, automated test scripts) is a thin HTTP client consuming the `/__kaanal/*` control API. No interface contains business logic (rule R2.6).
7. **Fail Fast and Loud**: Invalid configurations, unparseable schemas, cyclic topologies, and illegal overrides are rejected during startup with `file:line:col` error diagnostics (rule R6.1). Kaanal never serves half-configured states.

---

## 2. Crate Decomposition & Layering

The codebase is organized into four distinct crates with a strict one-way dependency hierarchy:

```mermaid
flowchart TD
    CLI["kaanal (Binary / CLI)<br/><code>crates/kaanal</code>"]
    Server["kaanal-server (HTTP Engine)<br/><code>crates/kaanal-server</code>"]
    Core["kaanal-core (Pure Logic & Topology)<br/><code>crates/kaanal-core</code>"]
    Data["kaanal-data (OpenAPI & Synthesis)<br/><code>crates/kaanal-data</code>"]

    CLI -->|Invokes serve/check| Server
    Server -->|Evaluates call chains & topology| Core
    Server -->|Generates mock JSON responses| Data

    style CLI fill:#f4f4f4,stroke:#333,stroke-width:2px
    style Server fill:#e1f5fe,stroke:#0288d1,stroke-width:2px
    style Core fill:#e8f5e9,stroke:#388e3c,stroke-width:2px
    style Data fill:#fff8e1,stroke:#f57f17,stroke-width:2px
```

### Dependency Rules and Invariants

| Crate               | Primary Role                                                                                       | Prohibited Technologies                                              | Invariant                                                                                 |
| ------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| **`kaanal-core`**   | Topology graph validation, latency distributions, chain execution, FNV-1a hash.                    | No `tokio`, no `axum`, no filesystem/network IO, no wall-clock time. | Pure computation only; computes outcomes, never sleeps. (Rule R2.3)                       |
| **`kaanal-data`**   | OpenAPI 3.0 parsing, `$ref` inlining, deterministic schema-driven data generator.                  | No dependency on `kaanal-core`, no HTTP, no async.                   | Maintains private FNV-1a hash algorithm pinned by cross-crate unit tests. (Rule R2.4)     |
| **`kaanal-server`** | Axum router, mock request handler, control API, SSE event bus, shared state.                       | No CLI terminal logic, no direct argv parsing.                       | Only crate that speaks HTTP. (Rule R2.4)                                                  |
| **`kaanal-sim`**    | Discrete-event simulator: Poisson arrivals, per-node FIFO queues, virtual clock.                   | No `tokio`, no IO, no timing model of its own.                       | Every service time comes from `kaanal_core::execute()`; depends on core only. (Rule R2.5) |
| **`kaanal`**        | CLI parsing via clap, directory locking, runfile management, `up -d` / `down`, control API client. | No direct mock engine or database logic.                             | Thin client over control API. (Rule R2.6)                                                 |

---

## 3. Topology Compilation & Graph Invariants

Kaanal models a system as a directed graph $G = (V, E)$, compiled during startup by `Topology::build`:

- **Nodes ($V$)**:
  - `Gateway`: The single entry point of the system (exactly 1 required).
  - `Service`: Backend service hosting an OpenAPI 3.0 `spec`.
  - `Database`: Data store providing `ops` (`read`, `write`, `scan`) costs.
- **Edges ($E$)**:
  - Directed dependencies carrying network latency distributions (`net`).

```mermaid
flowchart LR
    GW["Gateway (nginx)<br/>proc: 1ms"] -->|net: 1ms| SVC["Service (app)<br/>spec: openapi.yaml<br/>proc: 5ms"]
    SVC -->|net: 1ms| DB["Database (db)<br/>ops: read/write/scan"]

    style GW fill:#f9f9f9,stroke:#666
    style SVC fill:#e3f2fd,stroke:#1976d2
    style DB fill:#e8f8f5,stroke:#117a65
```

### Semantic Graph Validation

During `Topology::build`, the graph is verified against strict rules:

1. **Optional Gateway**: At most one node may have `type: gateway`; with none, requests enter at the `service` or `monolith` that owns the route. More than one fails the load (rule R2.7).
2. **Edge Table** (R1.24, `callees` in the topology module): `gateway` may call `service`, `monolith`, `auth`; `service` and `monolith` may call `service`, `monolith`, `database`, `cache`, `external-api`, `auth`; `auth` may call `service`, `monolith`, `database`, `cache`, `external-api`; `database`, `cache` and `external-api` call nothing. Any other edge is rejected, naming what the caller may call. Config errors are positioned (file, line, column, node, field) by `diagnostics` on the error path only.
3. **Cycle Rejection**: The graph must be a strict DAG. Cycle detection executes an iterative depth-first search (DFS) with 3-color vertex marking (unvisited, active path, completed). Detected cycles fail with `TopologyError::Cycle` listing the cycle path (`cycle: a -> b -> a`).
4. **Reachability**: When there is a gateway, any `service` or `monolith` node specifying an OpenAPI `spec` must have an incoming edge from it. Unreachable services fail with `TopologyError::MissingGatewayEdge`.
5. **Node Attribute Affinity**: each kind is its own strict struct, so a field on the wrong kind (`ops` on a service, `spec` on a gateway) is a parse error. A `database` must end up with all three ops (its own or from a flavor preset).

---

## 4. Execution Engine & Cost Model

When a mock request arrives, Kaanal evaluates what that request experiences across the simulated architecture:

```mermaid
sequenceDiagram
    autonumber
    actor Client
    participant Router as Axum Router (matchit)
    participant Mock as Mock Handler (mock.rs)
    participant Core as Chain Executor (exec.rs)
    participant Chaos as Effective (Arc snapshot)
    participant Data as Data Generator (generate.rs)
    participant Bus as SSE Broadcast Bus

    Client->>Router: GET /users/42
    Router->>Mock: Dispatched Route (its compiled endpoint: chain, op, override, error_map)
    Mock->>Chaos: snapshot() (clone one Arc)
    Mock->>Core: execute(topology, chaos, endpoint, n, chain_seed)
    Note over Core: Pure Synchronous Evaluation:<br/>1. Gateway proc (1.0ms)<br/>2. Net hop (1.0ms)<br/>3. Service proc (5.0ms)<br/>4. Net hop (1.0ms)<br/>5. DB read op (4.2ms)
    Core-->>Mock: ChainOutcome { hops, total: 12.2ms, status: Ok }
    Mock->>Mock: one tokio::time::sleep(12.2ms) for the whole chain
    Note over Mock: Shaping: failure → error_map; success → override, else generated body
    Mock->>Data: generate(schema, "response", seed=hash(seed, node, path))
    Data-->>Mock: Synthetic JSON Payload
    Mock->>Mock: apply_params(body, {"id": 42})
    Mock->>Bus: send(Event { status: 200, ms: 12.2, nodes })
    Mock-->>Client: 200 OK + X-Kaanal-Trace Header
```

### Operation Inference

The database hop operation (`Op`) is inferred from the HTTP request method and path template:

- `GET` or `HEAD` with a path parameter (e.g. `/users/{id}`) $\rightarrow$ `Op::Read` (single item lookup).
- `GET` or `HEAD` without parameters (e.g. `/users`) $\rightarrow$ `Op::Scan` (table/collection scan).
- All other HTTP methods (`POST`, `PUT`, `DELETE`, `PATCH`) $\rightarrow$ `Op::Write`.

### Latency Distribution Sampling

Node processing (`proc`), network hops (`net`), and database operations (`ops`) use `Dist`:

- **`Fixed(Duration)`**: Constant latency.
- **`LogNormal { mu, sigma, cap_ms }`**: Long-tailed distribution fitted to configured percentiles $\{p50, p99\}$:
  $$\mu = \ln(p50)$$
  $$\sigma = \frac{\ln(p99 / p50)}{Z_{99}} \quad \text{where } Z_{99} \approx 2.3263479$$
- **`Exp { mean }`**: Exponential (the queueing-theory default): $\text{mean} \times -\ln(1 - U)$ with $U$ uniform in $[0, 1)$, so $\ln(0)$ cannot occur.
- **Tail Cap Safety**: Lognormal samples are clamped to $\min(\text{sample}, 10 \times p99)$ and exponential samples to $10 \times \text{mean}$. This prevents astronomical tail samples from indefinitely hanging a request.
- **`concurrency`** (per node, optional) is not a distribution: it is a capacity for the node's own hops. `Node.admission` carries it to the replay engine in core, which both the simulator and live mode run: live mode queues the same way (one engine actor task), so a full pool makes requests wait (`node.wait=` in the trace) and a full bounded queue overflows.

**Second driver.** `kaanal-sim` calls the same `execute()` per simulated request and replays the returned `Plan` (the sampled tree of hops, forks and failures, E14) through the replay `Engine` in core (per-node FIFO queues, bounded queues with an overflow status, and token-bucket rate limits) on a virtual clock; live mode hands each sampled plan to the same engine on a real-time actor task. `parallel` blocks fork and join, and the first failure ends a request (running siblings keep their slot, queued ones leave). At zero load its latencies equal live mode's exactly; `kaanal-sim/tests/sim.rs` pins that.

### Chain Engine

- **Compile at load.** `compile` turns the spec routes and the `endpoints:` entries into one `Endpoint` per route. Default bodies are built callees-first and shared through `Arc`s; every error is placed in `kaanal.yaml` or in the endpoints file.
- **Execute per request.** `execute` walks the compiled chain as a timeline and returns the flat hop list, a `Plan` (the sampled tree) and the result.
- **Two drivers.** The server hands each plan to the live engine actor and shapes the response; the simulator replays the plan through its queues with tokens.
- **Seeding.** Each hop draws from the request's chain seed and a per-hop key, so a hop's numbers do not depend on which siblings ran.

### Auth

- **Config rules in core.** `check_auth` runs after parsing, so every error is a `Located` with a `line:col` (claims, scopes, limits, empty username); `Topology::build` allows one auth node and accepts an auth-only topology.
- **Built-in routes are ordinary endpoints.** `compile` adds `POST /login` and `GET /.well-known/jwks.json` as jobs owned by the auth node, tagged with a `Responder` (plain data). Chaos, trace, `error_map`, `endpoints:` customisation and simulator streams therefore need no special case. Core never builds tokens.
- **Server.** `auth.rs` holds the signing key (RS256, RFC 7638 `kid`, JWKS), the `Issuer` (users, claims, ttl, `iss`), the login body reader and the responder. In `mock::handle` a login body is read first (no sleep), then `sample_plan`, the engine, and the answer: chain failure through `error_map` first, then an `override`, then the responder.
- **Binary.** `keys.rs` owns key IO: location (`KAANAL_CONFIG_DIR` or the per-user local config folder), identity (hash of the config directory), race-safe store (temp file, hard link, adopt the winner), permissions, and the eager load at `up`; `validate` never touches keys.
- **Why this split.** Core stays free of IO and crypto (R2.3); the server owns crypto and HTTP; the binary owns the filesystem (X5). A config with an auth node and no key makes `serve` fail closed.

---

## 5. Live Chaos Engine & Upstream Propagation

Chaos settings live in `AppState.chaos` as an in-memory `std::sync::RwLock<Arc<Effective>>`: the stack of active scenarios plus the live layer, **compiled on every write** into an `EffectiveChaos`. A request clones the `Arc` once and uses that one snapshot; control writes take one `Mutex<Control>`, fold, validate and swap the `Arc`, so readers never wait for a fold. Chaos is never persisted to `kaanal.yaml` (scenarios may be defined there or in `scenarios/*.yaml`, but activation and live changes are runtime state) and does not require server restarts. The model is in [chaos.md](chaos.md).

```mermaid
flowchart LR
    subgraph Control["Control Plane"]
        CLI["kaanal chaos set"] -->|PUT /__kaanal/chaos/{node}| Guard["Security Guard"]
        UI["Web UI"] -->|PUT /__kaanal/chaos/{node}| Guard
        Guard --> Handler["set_chaos Handler"]
        Handler -->|"Control mutex: compile, swap Arc"| State[("RwLock<Arc<Effective>>")]
    end

    subgraph Runtime["Mock Request Pipeline"]
        MockHandler["mock::handle"] -->|"snapshot: one Arc clone"| State
        MockHandler --> Exec["kaanal_core::execute"]
        Exec --> Output["X-Kaanal-Trace & Error Status"]
    end
```

### Supported Chaos Controls

Per node, users can configure:

- `latency_ms`: Additional delay injected on every visit to that node (capped at 300,000 ms).
- `error_rate`: Probability $[0.0, 1.0]$ of failure on visit.
- `error_status`: HTTP error status code ($500 \le \text{status} \le 504$).
- `down`: Hard outage flag ($100\%$ failure with HTTP 503).

### Invariant: Unconditional Random Draws

To ensure determinism (rule R4.1), **the chaos error roll is always drawn from the RNG**, regardless of whether chaos is currently configured on that node. If the draw were skipped when chaos was inactive, enabling chaos would shift all subsequent PRNG draws, altering downstream latencies and body seeds. Drawing unconditionally guarantees that timing sequences remain identical across test runs.

### Failure Cascading & Tracing

When a downstream node fails (e.g., database error 504):

1. The execution chain terminates immediately at that hop.
2. The overall request adopts that node's error status.
3. Total delay includes all hops executed up to and including the failed node.
4. The response includes `X-Kaanal-Trace` ending in `node.error=STATUS` (e.g. `nginx.proc=1.0ms,net=1.0ms,app.proc=5.0ms,net=1.0ms,db.error=504`).

---

## 6. Deterministic Data Synthesis Engine

Response bodies are synthesized by `kaanal-data` using response schemas declared in OpenAPI 3.0 files.

```mermaid
flowchart TD
    ConfigSeed["kaanal.yaml seed"]
    
    subgraph Execution["Execution PRNG (kaanal-core)"]
        ExecSeed["per hop: stable_hash(path, node, tag); path = H(seed, endpoint_key, n) extended per step"]
        ChaChaExec["ChaCha8Rng (Latency & Chaos)"]
    end

    subgraph DataGen["Data Synthesis PRNG (kaanal-data)"]
        BodySeed["stable_hash(seed, node_name, path)"]
        Field1["child_seed(parent, 'id', 0)"]
        Field2["child_seed(parent, 'email', 1)"]
        Field3["child_seed(parent, 'address', 2)"]
        GroupSeed["Object Group Seed<br/>(Shared Place Context)"]
    end

    ConfigSeed --> ExecSeed --> ChaChaExec
    ConfigSeed -.-> BodySeed
    BodySeed --> Field1
    BodySeed --> Field2
    BodySeed --> Field3
    BodySeed --> GroupSeed
```

### Seeding Invariants

- **Endpoint Seed Independence (Rule R4.4)**: Each generated property derives its seed via:
  $$\text{child\_seed} = \text{stable\_hash}(\text{parent\_seed}, \text{field\_name}, \text{index})$$
  Adding, removing, or reordering schema properties never alters the generated values of sibling properties.
- **Geographic Coherence**: All address components (`city`, `state`, `pincode`, `street`) in a single JSON object sample a shared `group` seed derived from the enclosing object. A generated object never mixes a Mumbai pincode with a Chennai city.
- **Formats Beat Lengths (Rule R4.5)**: Standard formats (`uuid`, `email`, `date-time`, `ipv4`, `uri`) and well-formed heuristic fields are never truncated by `maxLength` or padded by `minLength`. A truncated email is invalid; well-formed formatting takes precedence.
- **URL Parameter Binding**: Path parameters (e.g., `/users/{id}` called as `/users/42`) overwrite matching schema properties via `apply_params`, safely coercing numeric types without crashing on non-numeric strings.

---

## 7. Control Plane, Security & Event Bus

The control plane exposes operational endpoints under `/__kaanal/*`:

```mermaid
flowchart LR
    subgraph Ingress["Single Port Ingress (e.g. :8080)"]
        Req["HTTP Request"] --> Guard{"Host & Token Guard"}
    end

    Guard -->|Host != Loopback| R403["403 Forbidden"]
    Guard -->|Invalid Token| R401["401 Unauthorized"]
    Guard -->|Valid Request| Dispatcher{"Route Dispatcher"}

    Dispatcher -->|GET /__kaanal/health| Health["Health Probe (No Auth)"]
    Dispatcher -->|GET /__kaanal/ui| UI["Status page (No Auth)"]
    Dispatcher -->|GET /__kaanal/topology| Topo["Topology Manifest"]
    Dispatcher -->|GET,PUT,DELETE /__kaanal/chaos| Chaos["Chaos State API"]
    Dispatcher -->|GET /__kaanal/events| SSE["SSE Broadcast Stream"]
```

### Security Guard Architecture

The `guard` middleware executes before any axum request extractor:

1. **Host Header Verification (DNS Rebinding Guard, Rule R5.3)**: `Host` must match `localhost`, `127.0.0.1`, or `[::1]`, optionally followed by the exact bound port. Requests targeting attacker domains resolving to loopback are rejected with 403.
2. **Bearer Token Authentication (Rule R5.2)**: All control routes (except `health` and `ui`) require `Authorization: Bearer <token>`. Comparisons use `constant_time_eq` to prevent timing attacks.
3. **Query Parameter Token Exemption**: `?token=<token>` is accepted exclusively on `GET /__kaanal/events` because standard browser `EventSource` cannot set custom headers.
4. **CORS Boundary (Rule R5.4)**: `Access-Control-Allow-Origin: *` is set exclusively on mock routes. Control endpoints never emit CORS headers, preventing rogue websites from manipulating chaos.
5. **Security Response Headers**: Every control response carries `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.

---

## 8. Status Page Architecture

Served at `/__kaanal/ui`, the status page is a self-contained health dashboard embedded in the binary via `include_str!` (`ui.html`). Studio (`/__kaanal/studio`) is the real UI; this page is what answers when nothing needs the token, and what the Studio address answers in a binary built without the Studio bundle.

- **No token, one call**: the page asks only `GET /__kaanal/health` (`ok`, `version`, `uptime_s`, `system`), every two seconds (ten while hidden). The route says nothing a token guards.
- **Fragment token pass-through**: a token in the URL fragment (never sent to a server) is only copied onto the "Open Studio" link, which opens Studio on the Admin tab when a project is loaded.
- **XSS immunity**: server values are shown through `textContent` only, never `innerHTML` or `eval()`.
- **Content Security Policy (CSP)**: served with `default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'`.
- **Bounded resource use**: the answer-time chart keeps the last 60 samples.

---

## 9. Process Lifecycle & Concurrency Model

```mermaid
flowchart TD
    Start(["kaanal up"]) --> PrivDir["Create .kaanal/ Directory<br/>(0700 Unix / icacls Windows)"]
    PrivDir --> Lock["Acquire OS File Lock<br/>(.kaanal/lock via File::try_lock)"]
    Lock --> CheckLock{Locked?}

    CheckLock -- No --> ExitBusy["Print 'already running' & Exit 1<br/>(up -d: exit 0)"]
    CheckLock -- Yes --> Init["Build AppState & Bind TCP Port"]
    Init --> RunFile["Atomically Write .kaanal/run.json<br/>(temp file + rename)"]
    RunFile --> Serve["Serve Requests on Tokio Multi-thread Pool"]

    Serve --> SigWait{"Wait for a stop signal<br/>or POST /__kaanal/shutdown"}
    SigWait --> Drain["Drain: refuse new connections,<br/>finish in-flight requests (5 s default),<br/>then cut what is left"]
    Drain --> TokenCheck{"run.json contains our token?"}
    TokenCheck -- Yes --> Unlink["Delete .kaanal/run.json"]
    TokenCheck -- No --> Keep["Leave run.json intact"]
    Unlink --> KernelUnlock["Process exits -> OS releases .kaanal/lock"]
    Keep --> KernelUnlock
    KernelUnlock --> Terminate(["Exit 0"])
```

`kaanal up -d` and `kaanal down` add two things around this: a second lock, `.kaanal/start.lock`, that serialises concurrent `up -d` runs, and a log, `.kaanal/kaanal.log`, for the detached process.

### Single Instance Enforcement

- **OS File Lock (Rule R6.7)**: `kaanal up` opens `.kaanal/lock` and calls `std::fs::File::try_lock()` (Rust 1.89+). If another instance holds the lock, it retries for about 200 ms (a status probe holds the lock for an instant) and then exits with code 1.
- **Crash Safety**: Operating system file locks are released by the kernel upon process termination. Stale lock files never occur after abnormal termination or `kill -9`.
- **Atomic State Persistence**: Server metadata (`host`, `port`, `token`, `pid` and a token-free `startup` object) is written to `.kaanal/run.json` via a temporary file and atomic rename.
- **The lock is the liveness signal**: a `run.json` whose lock is free is stale and is never trusted; every command that needs an instance checks the lock first (exit 2 and the file removed). `kaanal down` and `kaanal up -d` are built on the same check. User-facing description: [lifecycle guide](https://kaanal.naveenr.in/docs/guides/observe/lifecycle/); code: [files/kaanal/lifecycle.md](files/kaanal/lifecycle.md).
- **Graceful stop**: a signal or `POST /__kaanal/shutdown` starts a drain with a fixed deadline ([files/kaanal-server/shutdown.md](files/kaanal-server/shutdown.md)); the listener is wrapped so that connections still open at the deadline fail their I/O, which makes hyper drop the in-flight handlers.

---

## 10. System Invariants & 3 AM Failure Modes

| Potential Failure Mode                 | Root Cause                                                     | Kaanal Defense-in-Depth                                                                                 |
| -------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| **DNS-Rebinding Attack**               | Malicious website scripts browser to query `127.0.0.1:8080`.   | `guard` middleware checks `Host` header against loopback whitelist; rejects foreign hosts with 403.     |
| **Cross-Origin Chaos Manipulation**    | Browser tries to send preflight or POST to `/__kaanal/chaos`.  | CORS headers are strictly disabled on the control plane. Options preflights fail.                       |
| **RNG Desynchronization**              | Toggling chaos causes subsequent timings or payloads to shift. | Chaos error rolls are drawn unconditionally on every request, preserving downstream PRNG sequence.      |
| **Recursive `$ref` Memory Exhaustion** | Circular OpenAPI schema causes unbounded recursion.            | `ApiModel::from_openapi` tracks traversal stack and enforces a 10,000 reference expansion budget.       |
| **Race Conditions During Startup**     | Two terminal sessions launch `kaanal up` simultaneously.       | OS file lock on `.kaanal/lock` guarantees exactly one winner; loser fails cleanly with code 1.          |
| **Token Leakage via Proxy / Redirect** | CLI tool obeys `HTTP_PROXY` or follows malicious redirects.    | CLI `ureq` client explicitly disables system proxies and redirects.                                     |
| **Token Compromise in Logs**           | Token passed via query string appears in server or proxy logs. | Query string tokens are rejected everywhere except `/__kaanal/events`. UI reads token from `#fragment`. |
| **Port Clashing**                      | Configured port is already occupied by another service.        | Server automatically probes `port..=port+20`, binding the first available socket.                       |

## Code layout and file size

Source files stay small enough to read in one sitting. These are soft limits, a review trigger and not a compiler rule:

| File                                                 | Look for a seam | Split                     |
| ---------------------------------------------------- | --------------- | ------------------------- |
| Source (`src/**`, inline `#[cfg(test)]` code counts) | over 600 lines  | over 800 lines            |
| Integration tests (`tests/*.rs`)                     |                 | over 1000 lines, by topic |

A file may stay longer when it is one cohesive table or one type's impl; it then says why in a comment at the top. A split follows responsibility, not line count: the old module path stays (a folder module re-exports it, so callers do not change), the code is moved and not rewritten, and the matching page under `docs/source/files/` is updated in the same change.

Inside a crate, group files in folders by role instead of keeping a long flat `src/`. The `kaanal` binary is the model: `commands/` (one file per subcommand family; `demo/` and `init/` are folders), `instance/` (the running instance on disk and over HTTP) and `util/` (shared helpers), with `main.rs` and `cli.rs` at the top. Avoid a folder called `core`: it reads like the `kaanal-core` crate and shadows the standard library's `core` in paths. Folder modules are `pub(crate)`, and `main.rs` re-exports the few module names commands use by their short path.

A split must not widen what a crate exposes: items that were private become `pub(crate)` (or `pub(super)` inside a folder module), items that were `pub` stay as they were, and the old public paths keep working. A binary crate has no external API at all; for the library crates the plain-`pub` item set is compared before and after.

The limits are enforced in CI by `scripts/check-file-size.sh` (source files over 800 lines and test files over 1000 fail the build). A file that must stay larger, such as one type's hot-path `impl`, is listed with its reason in `scripts/file-size-allow.txt`; files over 600 lines only produce a notice.
