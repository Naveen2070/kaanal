# `crates/kaanal-server/src/pin_api.rs`

The pin control routes: `GET|PUT|DELETE /__kaanal/pins` and `POST /__kaanal/cycle`. Every write goes through `lock` / `commit` / `emit` of [control.md](control.md), like the chaos routes, so it is atomic with scenario writes (S11) and the publish clears the scoped cache. Reference for users: [control-api guide](https://kaanal.naveenr.in/docs/reference/control-api/).

## Handlers

| Handler       | Route                               | What it does                                                                                                                                                                                                                                                                  |
| ------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `get_pins`    | `GET /__kaanal/pins`                | `{ live: [{ endpoint, state, status?, delay_ms? }], effective: { KEY: { state, status?, delay_ms?, source } } }` from the published snapshot. `source` is `"live"` or `{ "scenario": NAME }`.                                                                                 |
| `put_pin`     | `PUT /__kaanal/pins`                | Sets the live pin of one endpoint; answers `{ endpoint, state, status?, delay_ms? }`. Emits a `scenario` event with action `pin`.                                                                                                                                             |
| `delete_pins` | `DELETE /__kaanal/pins?endpoint=EP` | Removes that live pin; with no query, clears every live pin. `204` also when there was nothing to remove, and then nothing is published and no event is sent. Scenario pins stay. Emits `pin` (name = the key, or `null`).                                                    |
| `cycle`       | `POST /__kaanal/cycle`              | Steps the endpoint's live pin with `PinState::next` (`empty` skipped where `empty_ok_for` says no form), or pins `state` when given. The next state is computed under the control lock, so concurrent cycles on one endpoint walk distinct consecutive states. Emits `cycle`. |

**Private:** `PinBody` (`endpoint`, `state?`, `status?`, `delay?`; unknown keys are rejected), `PinQuery`, `pinnable(st, raw)` (the normalized key and endpoint: `400` for a malformed key, a synthesized `HEAD` or a route with no entry node; `404` for an unknown one), `entry_of` (parses `delay` with humantime, runs `validate_entry` for the state, and refuses `empty` where `empty_ok_for` is false), `entry_json`, `answer`.

## Rules worth knowing

- Any `400` or `404` changes nothing, including a `cycle` whose `status` does not fit the state it lands on.
- `reset` clears the live pins through `ChaosStack::clear_all`; `DELETE /chaos` does not touch them.
- A scoped request never reads the live pins ([scope.md](scope.md)), so these routes do not disturb parallel workers.

## Tests

[../../tests.md](../../tests.md#crateskaanal-servertestspins_httprs--pins-over-http): `tests/pins_http.rs`.
