# `kaanal-core/src/spec_skeleton.rs`

`skeleton_openapi(entries, title) -> Result<String, String>`: an OpenAPI 3.0.3 YAML document for a map of `endpoints:` entries. Pure: it reads no file and knows no node. (Not `chain::skeleton`, which lists `endpoints:` lines for spec routes that have no entry.)

- Each key goes through `endpoint_key` (method upper-cased, path trimmed); a method OpenAPI 3.0 has no operation for (`TRACE`), a bad key, or the same method and path twice is an error naming the key. No entries is an error.
- The path is written unchanged: the loader gives spec paths to the router as they are, so `{*rest}` must stay a catch-all (a rewrite to `{rest}` would turn it into another route and the entry would be reported stale). Each distinct parameter name is declared once, `in: path`, `required: true`, `type: string` (OpenAPI requires `required: true`). An unbalanced brace or an empty name is an error naming the key.
- `operationId` is the method and the path words (`get_users_id`), made unique with a numeric suffix counted per base (linear, however many keys collide). At most `MAX_ENDPOINTS` = 10000 entries.
- The success response is the entry's `override` status (clamped to 100..599) or 200, with an `application/json` schema `{ type: object }`; a 204 or a `HEAD` has no body. Root key `x-kaanal-skeleton: true`.

Why it exists: an `endpoints:` entry cannot create a route owned by a node without a spec (R1.32), so Studio registers this file as a node's `spec:`. Tests: `tests/skeleton.rs` (structure) and `kaanal-server/tests/sim_http/skeleton.rs` (the output loads with `ApiModel::from_openapi`).
