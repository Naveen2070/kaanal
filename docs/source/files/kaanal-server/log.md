# `crates/kaanal-server/src/log.rs`

The request log: short ids, a bounded ring of finished requests, and the query filters. Pure data and logic (no `.await`, no HTTP types). `mock.rs` builds a `LogEntry` per request (through [record.md](record.md)) and pushes it, [log_api.md](log_api.md) reads the ring, and the CLI reuses `Filter` for `logs -f`. User reference: [observability guide](https://kaanal.naveenr.in/docs/guides/observe/observability/).

## Two orders, on purpose

`seq` is **arrival** order: the number in the id, taken when the request starts. `done` is **completion** order: assigned by the ring on push. A slow request has a low `seq` and a high `done`. So everything that must be ordered in time (the ring's eviction, `after=`, `logs -f`, `last`) uses `done`, and "is this id gone or not issued?" uses membership plus a high-water mark, never `seq < oldest`.

## Constants

| Item        | Value  | Meaning                                          |
| ----------- | ------ | ------------------------------------------------ |
| `LOG_CAP`   | 1000   | Entries the ring keeps (the last N completions). |
| `MAX_HOPS`  | 512    | `HopRec`s kept per entry; more sets `truncated`. |
| `MAX_TRACE` | 16 KiB | Bytes of trace-header text kept per entry.       |

## Ids

| Item                   | What it does                                                                                                                                                                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `IdGen`                | `boot: u16` (random once per start, `with_boot` for tests) and an `AtomicU64` counter starting at 1. `next()` returns `(seq, id text)` with a relaxed `fetch_add`: unique, not ordered against anything. Never reset, not even by `POST /reset`. |
| `format_id(boot, seq)` | `r04f2-0042`: `r`, four **zero-padded** lowercase hex digits (`{boot:04x}`), `-`, the number zero-padded to at least 4 digits (it grows past 9999). A three-digit boot such as `r4f2-0042` is not an id.                                         |
| `parse_id(text)`       | The inverse, grammar `^r[0-9a-f]{4}-[0-9]{4,}$`, to `(boot, seq)`. Uppercase hex, a short boot, a short number or a non-digit is `None`. Parsing is numeric, so `r04f2-00042` and `r04f2-0042` are the same id.                                  |
| `client_id(raw)`       | A client `X-Request-Id` worth keeping: `^[A-Za-z0-9._-]{1,64}$`, else `None`. Control bytes, spaces, 65 bytes and empty values are all dropped, so an entry can never carry anything that could drive a terminal or break a log line.            |

## Entry types

| Item            | What it holds                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HopRec`        | One node's slice of the trace: `node`, `replica` (only for a replicated node), `kind` (`net`, `proc`, `op`, `hit`, `miss`, `delay`), `attempt` (the retry a hop belongs to, from 2; absent in a first attempt), `op` (`read`/`write`/`scan` for an `op` hop), `start_ms`, `dur_ms`, `wait_ms` (simulated), `lane` (0 for the main path, `1..` for the branches of a fork), `fork` (the fork's ordinal), `cut` (shorter than planned), `why` (`chaos +40ms`, `chaos hang`, `pin delay`, `queued 1.5ms`, joined by `,`). `lane` and `fork` name the **innermost** `parallel` only: nested blocks flatten. |
| `ChainErrorRec` | What the chain failed with before `error_map` and before a pin hid it: `node`, `status`, `kind` (`status`, `reset`, `malformed`, `timeout`) and `cause` (`chaos`, `rate-limit`, `overflow`, `deadline`).                                                                                                                                                                                                                                                                                                                                                                                                |
| `LogEntry`      | One finished request: `id`, `client_id`, `seq`, `done` (set by the ring), `ts_ms`, `method`, `path`, `endpoint`, `status`, `ms` (measured), `trace_ms` (simulated total), `scope` (`Some("")` for an empty header), `pin`, `flags`, `chain_error`, `failed_node` (what the client saw; `None` under a pin), `kind`, `rejected`, `trace` and `hops`, `truncated`. `Serialize` is the wire shape of `/logs` and `/trace`.                                                                                                                                                                                 |

Never in an entry: bodies, request headers, query strings.

## Filters

`Filter` holds `endpoint`, `status: Vec<StatusMatch>`, `failed`, `scope: Option<String>`, `since_ms` and `after_done`. `StatusMatch` is `Exact(u16)` or `Class(u8)` (`5xx`).

`Filter::from_params(&[(k, v)], now_ms)` builds one from query pairs, **all or nothing**: an unknown key, a repeated key, an empty value or a malformed value is an `Err` whose message names the parameter. Rules:

- `endpoint` goes through `kaanal_core::scenario::endpoint_key` (method upper-cased, spacing normalised, must be `METHOD /path`). The entry's `endpoint` is the route template as registered, so `GET /users/1` is a valid key that no entry has: it matches nothing. That is deliberate and silent.
- `status` is a comma list of `404`, `5xx` (`1xx` to `5xx`, any case of `x`) and codes 100 to 599. Digits only; a sign or a space is an error.
- `failed` is `true` or `false` (`false` is "everything").
- `scope` is the one parameter that may be empty: `scope=` is `Some("")`, the scope of an empty `X-Kaanal-Scenario` header, distinct from "no filter" and from entries with no header. It is matched exactly against the key the API echoes (the names joined by `,` in header order); `scope.rs` has no string canonicaliser to reuse.
- `since` is a `humantime` duration (turned into `now - duration`) or an RFC 3339 time.
- `after` is plain digits (`str::parse` alone would accept `+5`).
- `limit` is accepted and ignored here; [log_api.md](log_api.md) parses and bounds it (1 to 1000).

`matches(entry, filter)` is the single predicate. A filter on `failed` looks at `failed_node`, so a pinned request that hid a failure does not match.

## `LogRing`

| Item                    | What it does                                                                                                                                                                                                                                                           |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `push(entry) -> done`   | Assigns `done` (1, 2, ...), raises the high-water mark to `max(seq)`, appends, and drops the **earliest completion** past `LOG_CAP` (counted in `evicted`). The lowest `seq` is not what goes: a late slow request stays until 1000 later ones finish.                 |
| `clear()`               | Empties the entries; `done`, the high-water mark and `evicted` keep counting, so every id issued before is `Evicted`, not `Unknown`, and `done` stays unique for `logs -f`. Called by `POST /reset`.                                                                   |
| `query(&filter, limit)` | The last `limit` matches in completion order (with `after` set, the first `limit` after the cursor), as a `Page { entries, oldest, newest, cap, evicted, boot }`. `oldest`/`newest` are the ids with the lowest/highest `seq` **in the ring**; both `None` when empty. |
| `find(key) -> Lookup`   | Precedence: the literal `last` (highest `done`), then one of our ids, then a client id (the newest match). So a client id shaped like an id resolves as an id. See below.                                                                                              |

`Lookup`:

- `Found(entry)`.
- `Evicted`: this boot's id, number at or below the highest recorded, but not in the ring. It also covers an id still in flight whose number is below a finished one (the detail text says "or still in flight").
- `Unknown`: another boot's id, number 0, a number above everything recorded, a client id nobody sent, or `last` on an empty ring.
- `Malformed`: neither an id nor a valid client id.

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrclogrs-unit--ids-the-ring-and-the-filters): 18 unit tests in the file.

**`edited` (step 8A).** `LogEntry.edited` is true when a live [payload edit](payload.md) answered the request; it is left out of the JSON when false.
