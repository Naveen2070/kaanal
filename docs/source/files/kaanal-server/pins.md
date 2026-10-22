# `crates/kaanal-server/src/pins.rs`

The one decision of what an `empty` pin answers, and the two adapters that let `resolve_scenario` ask it at load time and at runtime, so load time and request time cannot disagree. The pin routes are in [pin_api.md](pin_api.md); the front door that uses the result is in [mock.md](mock.md).

## Items

| Item                                                  | Kind | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------- | ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `route_seed(seed, owner, path) -> u64`                | fn   | The seed of a route's generated data: `stable_hash(seed, owner, path)`. `generated_body` and the `empty` body share it, so generated scalars are the same in both.                                                                                                                                                                                                                                                                                                                                                                 |
| `empty_form(ep, schema, seed, opts) -> Option<Value>` | fn   | The body an `empty` pin answers, or `None` when the endpoint has none. In order: `states.empty.body` (verbatim); an endpoint with a built-in responder and no `override:` has none; the emptied `override:` body (`empty_of_value`); the emptied response schema (`kaanal_data::empty`, with `opts`: the mock passes `options_for_owner(ep.owner)`, so the owning node's locale and `data.yaml` apply; the load-time and runtime `empty_ok` checks pass the default options, since whether a form exists does not depend on them). |
| `empty_ok(info, seed)`                                | fn   | The load-time check: a closure over the spec table (key to status and schema), because routes do not exist yet. A `HEAD` with no entry of its own reads its `GET`'s.                                                                                                                                                                                                                                                                                                                                                               |
| `AppState::empty_ok_for(&Endpoint)`                   | fn   | The runtime check, from the built route (`PUT /scenarios`, `scenarios/save`, pins, cycle). A route that cannot be found has no form.                                                                                                                                                                                                                                                                                                                                                                                               |

**Private:** `has_form(ep, normal_status, schema, seed)`: a route that normally answers **204 or 304** and has no responder always has an empty form ("no body", the status stays); otherwise `empty_form` decides.

## Rules worth knowing

- **Built-in routes** (login, JWKS) have no empty form unless the entry gives `states.empty.body` or an `override:` with a body; `cycle` then skips `empty` for them.
- A spec route with an `override:` derives `empty` from the override body, not the schema (the override is what the client sees).
- Heuristic ceiling: see [../kaanal-data/empty.md](../kaanal-data/empty.md).

## Tests

[../../tests.md](../../tests.md#crateskaanal-servertestspins_httprs--pins-over-http): `tests/pins_http.rs`.
