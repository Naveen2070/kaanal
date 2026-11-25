---
title: "Pins"
sidebar:
  order: 3
---

Live [pins](/docs/guides/break/pins/) force an endpoint to answer as `loading`, `empty`, `error`, `slow` or `success`. They sit above every scenario's pins. All four routes need the token, and every write is atomic like the scenario routes.

| Method and path                     | Purpose                                                                  | Success                                               |
| ----------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------- |
| `GET /__kaanal/pins`                | Live pins and the pin in force per endpoint                              | `200 { "live", "effective" }`                         |
| `PUT /__kaanal/pins`                | Set the live pin of one endpoint                                         | `200 { "endpoint", "state", "status"?, "delay_ms"? }` |
| `DELETE /__kaanal/pins?endpoint=EP` | Remove that live pin; with no query, every live pin. Scenario pins stay. | `204` (also when there was nothing to remove)         |
| `POST /__kaanal/cycle`              | Step the endpoint's live pin to the next state, or pin `state`           | `200`, same shape as `PUT`                            |

**Body** of `PUT /pins` and `POST /cycle`:

```json
{ "endpoint": "GET /orders", "state": "error", "status": 429, "delay": "500ms" }
```

- `endpoint` is required (`METHOD /path`, method case-insensitive). `state` is required for `PUT` and optional for `cycle` (none: the next of `loading, empty, error, slow, success` after the endpoint's current live pin, wrapping, skipping `empty` where it has no empty form). `status` and `delay` (text such as `"2s"`) are optional. Unknown keys are `422`.
- Rules: `success` takes no `status` or `delay`; `status` is 400 to 599 on `error`, 200 to 299 on `empty`, not allowed on `loading` or `slow`; `delay` is above 0 and at most 5 min.

**`GET /pins`**:

```json
{
  "live": [ { "endpoint": "GET /orders", "state": "error", "status": 429, "delay_ms": 500 } ],
  "effective": {
    "GET /orders": { "state": "error", "status": 429, "delay_ms": 500, "source": "live" },
    "GET /users": { "state": "empty", "source": { "scenario": "orders-empty" } }
  }
}
```

`effective` is the winning pin per endpoint (a `success` pin is listed, because it masks); `source` is `"live"` or `{ "scenario": NAME }`.

| Status | When                                                                                                                                                                                      |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | Malformed endpoint, a `HEAD` made from its `GET` (pin the `GET`), a route with no entry node, unknown `state`, a `status` or `delay` the state does not allow, `empty` with no empty form |
| `404`  | Unknown endpoint                                                                                                                                                                          |
| `422`  | The body is not a pin (unknown key, missing `endpoint`)                                                                                                                                   |

A `400` or `404` changes nothing. `POST /__kaanal/reset` clears live pins; `DELETE /__kaanal/chaos` leaves them.
