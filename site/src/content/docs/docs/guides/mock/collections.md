---
title: "Stateful routes: collection:"
sidebar:
  order: 4
---

Most mock routes answer with generated data that never changes. A **collection** remembers: a `POST` adds an item, a `GET` lists what was added, a `DELETE` removes it. Enough for a cart, a todo list or a notes page to behave like the real thing while you build the frontend.

```yaml
endpoints:
  "GET /cart":         { collection: cart }
  "POST /cart":        { collection: cart }
  "GET /cart/{id}":    { collection: cart }
  "DELETE /cart/{id}": { collection: cart }
```

Each route still needs to be in your OpenAPI spec. Routes that name the same collection share its items.

## The key

`collection:` goes on an [`endpoints:`](/docs/guides/mock/endpoints/) entry. The name is 1 to 32 characters of `a-z`, `0-9`, `_` or `-`, starting with a letter. It cannot be combined with `override:`. The route must have one of the four shapes below; anything else is an error at load time. The path is one top-level segment, optionally followed by one `{param}`: `/cart` and `/cart/{id}` work, `/api/cart` does not.

## The four shapes

| Route               | Does                           | Answers                                            |
| ------------------- | ------------------------------ | -------------------------------------------------- |
| `GET /cart`         | Lists every item, oldest first | `200` and a JSON array (`[]` when empty)           |
| `POST /cart`        | Adds the request body          | `201` and the item with its new `id`               |
| `GET /cart/{id}`    | Reads one item                 | `200` and the item, or `404 {"error":"not found"}` |
| `DELETE /cart/{id}` | Removes one item               | `204` with no body, or `404 {"error":"not found"}` |

A `HEAD` of a `GET` route answers like the `GET`, with no body. It never changes anything.

```console
$ curl -s -X POST localhost:8080/cart -H 'content-type: application/json' -d '{"sku":"A-1","qty":2}'
{"id":1,"qty":2,"sku":"A-1"}
$ curl -s localhost:8080/cart
[{"id":1,"qty":2,"sku":"A-1"}]
$ curl -s -X DELETE localhost:8080/cart/1 -o /dev/null -w '%{http_code}\n'
204
```

## Items and ids

- An item is a JSON object. Anything else (an array, a string, invalid JSON) is `400 {"error":"item must be a JSON object"}`.
- The `id` is a whole number given by Kaanal, counting from 1 in each collection. An `id` you send is replaced. A removed id is never handed out again (until a reset).
- An id in the URL must be written plainly (`7`, not `007` or `+7`); anything else, or an unknown id, is a 404.
- A stored item comes back with its keys in alphabetical order, as the example above shows, not in the order you sent them.
- There is no update (`PUT`/`PATCH`), filter or paging. (Routes that _generate_ a list from your spec can be paged: see [paging](/docs/guides/mock/paging/).)

## Limits

| Limit                      | Value  | Over it                           |
| -------------------------- | ------ | --------------------------------- |
| Items per collection       | 1000   | `409 {"error":"collection full"}` |
| Size of one item (as JSON) | 64 KiB | `413 {"error":"item too large"}`  |

The size is measured on the item about as you sent it (before its `id` is added), so the limit is approximate by a few bytes. The limits are fixed. A rejected add changes nothing and uses no id.

## It still goes through the simulation

A collection route is a normal route: its chain runs first, with the latency, queues, rate limits and chaos of every node it visits. So a slow database makes the cart slow, and you can watch your frontend's loading state.

The item is stored (or removed) **only when the request succeeds**:

- a chaos failure (`503` from the database, a timeout) stores nothing;
- a `reset` or `malformed` failure on the wire shows the answer the request would have given, cut off, and changes nothing. An answer with no body to cut (the `204` of a `DELETE`, any `HEAD`) is instead a chaos error, `502 {"error":"chaos","node":...,"kind":...}` as on any other node (or what your `error_map` says for that status), so a client never sees a clean success the collection contradicts;
- a [pin](/docs/guides/break/pins/) to `error` or `empty` changes nothing; a pin to `loading`, `slow` or `success` answers normally and applies the change once. A `DELETE` pinned to `empty` answers its usual `204` and removes nothing.

A bad body (not an object, too large) is answered only after the chain, so it takes the same time as a good one, and a chain failure wins over it.

## Shared, in memory

- One set of collections per running server, **shared by everyone**: every [`X-Kaanal-Scenario`](/docs/guides/break/scenarios/) scope and every logged-in user sees the same items. Mock routes do not check bearer tokens, so if your app has a per-user cart, it is one cart here.
- Items live in memory only. They are empty at start, gone on restart, and emptied by `kaanal reset` (`POST /__kaanal/reset`), which also restarts the ids at 1.
