# Architecture

## Crates

```
kaanal (bin) ──► kaanal-server ──► kaanal-core
                        └───────► kaanal-data
```

Dependencies go one way only:

- **`kaanal-core`** — pure logic. No tokio, no axum, no IO, no global state. Parses config, builds the topology graph, computes latency distributions, walks call chains. Deterministic given its inputs; it computes outcomes and never sleeps.
- **`kaanal-data`** — OpenAPI 3.0 parsing and deterministic mock-data generation. Never depends on core (it keeps its own copy of the FNV-1a hash for that reason).
- **`kaanal-server`** — the only crate that knows HTTP. Axum router, control API (`/__kaanal/*`), mock fallback handler, embedded HTML page, shared `AppState`.
- **`kaanal`** (bin) — the only crate that knows argv and the terminal. Thin `clap` CLI; every subcommand other than `up`/`validate` is an HTTP call to the control API.

## Request flow

```
kaanal up
  └─ main.rs: one tokio multi-thread runtime
       └─ kaanal_server::serve()
            ├─ bind TcpListener (requested port, else next free up to +20)
            ├─ AppState::build()   ← config, topology, OpenAPI specs, routes
            └─ tokio::spawn(axum::serve(listener, router))

Router (single port):
  /__kaanal/health|topology|chaos|events|ui   ── control.rs (guard middleware first)
  everything else (fallback)                  ── mock.rs
```

A mock request is handled entirely inline in `mock::handle`:

1. CORS preflight short-circuits (never on `/__kaanal/*`).
2. `matchit` looks up the path; method not matched → 405, no route → 404.
3. `kaanal_core::execute()` walks the call chain **synchronously** in the handler: gateway proc → net → service proc → net → database op. At each node it checks chaos (down, error rate, added latency). It is pure: it returns hop durations, it does not sleep.
4. `tokio::time::sleep(outcome.total())` makes the client actually wait the simulated time.
5. The body is a static override or generated from the endpoint's schema (`kaanal_data::generate`), seeded by `hash(node, path)` so the same URL always returns the same data. Path params are then copied into matching fields.
6. The `X-Kaanal-Trace` header carries the per-hop waterfall; an `Event` goes on the `broadcast` bus for `/__kaanal/events` (SSE) subscribers.

## Threading model

One tokio multi-thread runtime (worker threads = CPU cores). There is **no** separate engine thread: the engine (chain executor, data generator) runs inside the request handler's task on the shared worker pool. This is deliberate for phase one — the CPU work per request is microseconds. See `internal/docs/suggestions/2026-10-08.md` for the pressure points and the two-listener split if load isolation is ever needed.

## Shared state

`AppState` (one `Arc`, in `kaanal-server/src/state.rs`):

| Field                         | Type                                        | Mutated by                                                           |
| ----------------------------- | ------------------------------------------- | -------------------------------------------------------------------- |
| `topo`, `paths`, `route_keys` | built once at startup                       | never (immutable)                                                    |
| `chaos`                       | `std::sync::RwLock<ChaosState>`             | control API writes, mock handler reads (briefly, across `execute()`) |
| `events`                      | `tokio::sync::broadcast::Sender` (cap 1024) | mock handler sends per request                                       |
| `seed`, `token`, `port`       | plain values                                | never                                                                |

## Determinism model

- All randomness comes from `ChaCha8Rng` seeded by **FNV-1a 64** (`stable_hash`) over explicit parts — never std hashers (not stable across platforms/versions).
- Chain outcome seed = `hash(seed, endpoint_key, request_counter n)`. Same sequential request stream → same timings and failures every run.
- Generated body seed = `hash(node, path)`. Each field's seed derives from `(parent_seed, field_name, index)`, so adding or reordering schema properties never changes other fields.
- The chaos error roll is **always drawn**, even when no chaos is set, so enabling chaos never shifts the RNG sequence of timings.

## Security model (phase one)

- Binds `127.0.0.1` by default; non-loopback bind prints a warning.
- Control API: every route goes through the `guard` middleware — `Host` must be loopback with the bound port (DNS-rebinding guard), and every route except `health`/`ui` needs the 256-bit bearer token (`?token=` allowed only on `/events` for `EventSource`).
- CORS `*` on mock routes only; the control namespace never gets CORS.
- The token lives in `.kaanal/run.json`, created with user-only permissions (0700/0600 on Unix, a single-user ACL on Windows).
- One instance per project directory via an OS file lock on `.kaanal/lock`.
