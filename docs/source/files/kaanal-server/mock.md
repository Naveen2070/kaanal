# `crates/kaanal-server/src/mock.rs`

The mock request handler — the axum fallback that serves every non-control route.

## What it does

`handle(State, req)` turns one HTTP request into a mocked response, end to end:

1. **CORS** — preflight (`OPTIONS`) returns 204 with permissive headers. The `/__kaanal` namespace never gets CORS, not even on a 404 (rule R5.4).
2. **Routing** — `matchit` lookup by path; 404 `{"error":"no route"}` on miss. Within a matched path, only the exact method matches. A `HEAD` with no spec entry finds the `HEAD` route `AppState::build` synthesised from `GET` (own key and counter, rule R6.8; hyper strips the body). No method → 405.
3. **Chain execution** — builds a `ChainRequest` (owning service, inferred op, endpoint key, per-route atomic counter `n`, config seed) and calls `kaanal_core::execute` **synchronously in the handler** (microseconds of pure math). The chaos `RwLock` read guard is held across it — briefly.
4. **Sleep** — `tokio::time::sleep(outcome.total())` makes the client actually wait the simulated time. (Windows caveat: sub-15 ms sleeps round up to the OS timer tick; measured latency can exceed the simulated sum.)
5. **Trace header** — `X-Kaanal-Trace`: comma-separated hops with one decimal (`nginx.proc=1.0ms,net=1.0ms,db.read=1.0ms`); a failure appends `node.error=STATUS` naming the node whose chaos stopped the chain.
6. **Body** — on `ChainResult::Err`: `{"error":"chaos","node":name}` with the chaos status. On `Ok`: the static override if the route has one, else `kaanal_data::generate` seeded by `stable_hash(seed, node, path)` (same `seed:` + URL → same body forever; a new `seed:` reshuffles the data), then `apply_params` copies path params into matching fields.
7. **Event** — sends an `Event` (timestamp, method, path, status, **measured** wall ms, ordered unique node names, failing node) on the broadcast bus for SSE subscribers. No subscribers = send error, ignored by design.

## Other items

| Item              | What it does                                                                                                                                                                                                                |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `with_cors(resp)` | Adds `Access-Control-Allow-Origin: *`, methods, headers, and exposes `x-kaanal-trace`. Mock routes only: every return in `handle`, the final one included, goes through the `cors` closure, which skips it for `/__kaanal`. |

## Unit tests in this file

None; path params are decoded with `percent_encoding::percent_decode_str` (invalid escapes kept, invalid UTF-8 → U+FFFD). HTTP-level behaviour: [../tests.md](../../tests.md#crateskaanal-servertestshttprs).
