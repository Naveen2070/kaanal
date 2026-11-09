# `crates/kaanal-server/src/payload.rs` and `control/payload.rs`

Live response edits (step 8A, RULES 108): rewrite the status, headers and JSON body one route answers, while a frontend calls it. For users: [../../../web/admin-mode.md](../../../web/admin-mode.md); the routes: [../../../web/control-api.md](../../../web/control-api.md#payload-edits).

## `payload.rs`

| Item                                | Kind   | What it does                                                                                                                                                                                                                  |
| ----------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Payloads`                          | struct | The edits by normalized route key (`GET /users/{id}`), each a `kaanal_core::config::Override`. Copy on write: a request clones one `Arc` ([`AppState::payloads`]) and reads it, so a write never changes what a request sees. |
| `Payloads::get_for(ep)`             | fn     | The edit of an endpoint; a synthesized `HEAD` follows its `GET`, as pins do. Empty table: no allocation.                                                                                                                      |
| `PayloadError`                      | enum   | `NoRoute`, `Synthesized`, `Collection`, `BuiltIn`, `Invalid`; each holds the message shown.                                                                                                                                   |
| `AppState::set_payload(g, raw, ov)` | fn     | Normalizes the key, refuses a route that is unknown, a `HEAD` made from its `GET`, a `collection:` route or a built-in responder without `override:`, then runs `Override::check` and stores. Nothing is stored on a refusal. |
| `clear_payload`, `clear_payloads`   | fn     | One edit, or all (`POST /reset`).                                                                                                                                                                                             |
| `prune_payloads(g)`                 | fn     | Drops the edits of routes the new generation no longer takes one on; returns the keys. Called by `apply` under the control lock, before the swap.                                                                             |

## Where an edit is applied

[`mock/flight.rs::finish`](mock.md): after the reply is built, when the request is unscoped, nothing failed (no chain error, no wire failure) and no pin forced `error` or `empty`, the edit's status and body replace the reply's and its headers go on the response (a file `override:`'s headers are sent the same way). `paged` is off for an edited reply, and `mock::resp_bytes` counts the edit's body for size costs. The log entry carries `edited: true`.

## `control/payload.rs`

`list`, `put`, `delete` and `preview` for `GET|PUT|DELETE /__kaanal/payload` and `GET /__kaanal/payload/preview`. `put` reads the body with `body::read_capped` (cap `Override::MAX_BODY + 4096`: `413` above), parses `{ route, status?, body?, headers? }` and answers `404` (no route), `422 { error, field }` (the field is `status`, `body`, `headers` or `route`) or `204`. Writes take the control lock (`lock_gen` for `put`), so an Apply, which prunes under the same lock, never meets a half-set edit. Each write marks the live timeline (`payload`). `preview` is [`mock::preview`](mock.md): what the route answers with no edit.

## Tests

`tests/draft_http/payload.rs` (the routes, the cap, the token, the markers, the prune on Apply), `tests/http/payload.rs` (a login route refuses), `tests/http/admin_mode.rs` (a real request, an edit, a scoped request, back to normal), and `mock/payload_tests.rs` (precedence: chaos, pins, file override, scope, HEAD, sizes).
