---
title: "Payload edits"
sidebar:
  order: 4
---

Live edits rewrite a route's status, headers, and body (JSON, text with `content_type`, or base64 bytes with `encoding: "base64"` and `content_type`) starting from the next request. Use `once: true` for a single request.

Edits power Studio's [Admin mode](/docs/guides/break/admin-mode/), sit below chaos/pins and above `override:`, and are ignored when `X-Kaanal-Scenario` is set. All routes require the token.

| Method and path                            | Purpose                                                                                                                                       | Success                                                 |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `GET /__kaanal/payload`                    | The edits, in route order                                                                                                                     | `200 { "edits": [ { route, status, body, headers } ] }` |
| `PUT /__kaanal/payload`                    | Set the edit of one route                                                                                                                     | `204`                                                   |
| `DELETE /__kaanal/payload?route=KEY`       | Back to normal for one route                                                                                                                  | `204`                                                   |
| `GET /__kaanal/instances`                  | The cloud instance table `instance:` reads: `{ types: [[name, vcpu, memory_mib, net_mbps], ...] }`, sorted by name (`net_mbps` 0 = no figure) | `200`                                                   |
| `GET /__kaanal/prices`                     | The regions of each cloud the cost estimate can price in, `{ defaults, regions }`.                                                            |                                                         |
| `GET /__kaanal/prices/{provider}/{region}` | One region's prices, `{ provider, region, prices: [[name, usd_micro_hr], ...] }` (404 for an unknown provider or region).                     |                                                         |
| `POST /__kaanal/payload/stage-file`        | Write the edit (the body of `PUT`) into the endpoints file the draft names; `endpoints:` must be a file, not inline                           | `200 { "file" }`                                        |
| `GET /__kaanal/payload/preview?route=KEY`  | What the route answers with no edit (a starting text; path parameters are filled with `1`)                                                    | `200 { status, body, headers }`                         |

**Body** of `PUT`: `{ "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "retry-after": "5" } }`. `route` is required; `status` defaults to 200, `body` to none and `headers` to none. Unknown keys are `400`.

| Status | When                                                                                                                                                                                                                                                                                                                                                              |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | The body is not JSON or has an unknown key; a missing or malformed `route` in the query                                                                                                                                                                                                                                                                           |
| `404`  | No such route (`PUT`), no edit on the route (`DELETE`), unknown route (`preview`)                                                                                                                                                                                                                                                                                 |
| `413`  | The body is over 1 MiB                                                                                                                                                                                                                                                                                                                                            |
| `422`  | `{ "error", "field" }`: the edit is refused, and `field` is `status` (not 200 to 599, or a body on a `204` or `304`), `body` (over 1 MiB), `headers` (over 32, a bad name, a value over 4096 bytes or with a control character, or a name Kaanal sets itself) or `route` (a `collection:` route, a built-in login or key-set route, a `HEAD` made from its `GET`) |

- **`stage-file`:** validates like `PUT`, patches the external endpoints file atomically while preserving comments (`409` if `endpoints:` is inline, modified on disk, or invalid), and leaves the live edit active until the next Apply.
- **Refusals & resets:** failed `PUT`s change nothing; Apply keeps edits for matching routes and reports dropped ones in `payloads_dropped`; `POST /__kaanal/reset` clears all edits.
