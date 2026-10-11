# `crates/kaanal-server/src/mock.rs`

The mock request handler — the axum fallback that serves every non-control route.

## What it does

`handle(State, req)` turns one HTTP request into a mocked response, end to end:

1. **CORS** — preflight (`OPTIONS`) returns 204 with permissive headers. The `/__kaanal` namespace never gets CORS, not even on a 404 (rule R5.4).
2. **Routing** — `matchit` lookup by path; 404 `{"error":"no route"}` on miss. Within a matched path, only the exact method matches. A `HEAD` with no entry of its own finds the `HEAD` copy `compile` made from `GET` (own key and counter, rule R6.8; hyper strips the body). No method → 405.
   2a. **Login body** — a route whose responder is `Login` and that has no `override` has its body read now, before the chain (`auth::read_login`; no sleep yet), so a bad body still costs the full chain latency. An `override` replaces the responder, so its body is never read.
3. **Chain execution** — builds a `ChainRequest` (the route's compiled `Endpoint`; per-route atomic counter `n`; `AppState.chain_seed`) and calls `kaanal_core::execute` **synchronously in the handler** (microseconds of pure math). The chaos `RwLock` read guard is held across it — briefly.
4. **Sleep** — one `tokio::time::sleep(outcome.total())` for the whole chain makes the client actually wait the simulated time. (Windows caveat: sub-15 ms sleeps round up to the OS timer tick; measured latency can exceed the simulated sum.)
5. **Response shaping** — on `ChainResult::Err`, `resolve_error` looks the failure up in the endpoint's `error_map` (node name, then exact status, then class such as `5xx`); with no match the failure's own status and `{"error":"chaos","node":name}` are used. Precedence: chain failure (through `error_map`) > `override` > built-in responder (413/415/400, then 401 or 200; the JWKS is 200; a JWKS `HEAD` answers through its synthesized copy) > generated body. On `Ok`: the endpoint's `override` (status and body) if it has one, else the responder's answer (`auth::respond`), else `kaanal_data::generate` seeded by `stable_hash(seed, node, path)` (same `seed:` + URL → same body forever; a new `seed:` reshuffles the data; the chain seed is not involved), then `apply_params` copies path params into matching fields.
   5a. **No-store** — every response of a `Login` route (including chain failures and an `override`) gets `Cache-Control: no-store` and `Pragma: no-cache`. The JWKS does not.
6. **Trace header** — `X-Kaanal-Trace` (built by `trace_header`): the visible hops in plan order with one decimal (`nginx.proc=1.0ms,net=1.0ms,db.read=1.0ms`); cache lookups print `cache.hit=` or `cache.miss=`; a `parallel` block is bracketed with `|` between branches, e.g. `nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms`; a failure appends `node.error=STATUS` (the real failure, even when `error_map` changed the response status). Branches a failure cancelled show only what they ran before the failure instant (a branch that never started is empty). The value is capped at 4096 bytes: past that it stops before a hop, closes any open `[`, and ends `,...+N hops` with N the hops left out (ASCII, D13); the error part is always kept.
7. **Event** — sends an `Event` (timestamp, method, path, status, **measured** wall ms, ordered unique node names, failing node) on the broadcast bus for SSE subscribers. No subscribers = send error, ignored by design.

## Other items

| Item                          | What it does                                                                                                                                                                                                                |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `with_cors(resp)`             | Adds `Access-Control-Allow-Origin: *`, methods, headers, and exposes `x-kaanal-trace`. Mock routes only: every return in `handle`, the final one included, goes through the `cors` closure, which skips it for `/__kaanal`. |
| `trace_header(outcome, topo)` | Builds the `X-Kaanal-Trace` value from the `Plan` and the kept hops: brackets for `parallel`, `.hit`/`.miss` for caches, the error part, then the 4096-byte cap via `render`.                                               |
| `render(pieces, cap, tail)`   | Joins trace pieces; stops before the first hop that would not leave room for the open brackets, the `,...+N hops` suffix and the error part.                                                                                |
| `no_store(resp)`              | Adds `Cache-Control: no-store` and `Pragma: no-cache` (login routes).                                                                                                                                                       |

## Unit tests in this file

None; path params are decoded with `percent_encoding::percent_decode_str` (invalid escapes kept, invalid UTF-8 → U+FFFD). HTTP-level behaviour: [../tests.md](../../tests.md#crateskaanal-servertestshttprs).
