---
title: "Paged lists"
sidebar:
  order: 5
---

A list route in your OpenAPI spec that takes a paging parameter (`limit`, `page`, `cursor`, ...) answers with **one page** of a stable, generated dataset. Walk the pages and you see every record once; change the page size and you see the same records cut differently. Your pagination UI, infinite scroll and "load more" button can be built against it without a backend.

There is nothing to switch on. Kaanal reads the paging parameters your spec already declares. A route that declares none answers exactly as before.

```yaml
# openapi.yaml
paths:
  /pets:
    get:
      parameters:
        - { name: limit, in: query, schema: { type: integer, maximum: 50 } }
        - { name: offset, in: query, schema: { type: integer } }
      responses:
        "200":
          content:
            application/json:
              schema: { type: array, items: { $ref: "#/components/schemas/Pet" } }
```

```console
$ curl -si 'localhost:8080/pets?limit=2&offset=4'
HTTP/1.1 200 OK
x-total-count: 87
link: </pets?limit=2&offset=0>; rel="first", </pets?limit=2&offset=2>; rel="prev", </pets?limit=2&offset=6>; rel="next", </pets?limit=2&offset=86>; rel="last"
...
[{"id":...},{"id":...}]
```

## The three styles

| Style  | Index parameter (names, case, `_` and `-` ignored)               | Means                                |
| ------ | ---------------------------------------------------------------- | ------------------------------------ |
| page   | `page`, `pageNumber`                                             | page number, 1-based unless 0-based  |
| offset | `offset`, `skip`                                                 | 0-based position of the first record |
| cursor | `cursor`, `after`, `pageToken`, `nextPageToken`, `startingAfter` | an opaque token Kaanal gave you      |

The page size parameter is `limit`, `size`, `pageSize`, `perPage`, `count` or `maxResults`. It is optional: without it the page size is the default below.

Page numbers start at **1**, unless the page parameter says they start at 0: it declares `minimum: 0`, or `default: 0` with no `minimum` above 0. That is the Spring Data shape, and it works as is:

```yaml
parameters:
  - { name: page, in: query, schema: { type: integer, minimum: 0 } }
  - { name: size, in: query, schema: { type: integer } }
# response: { content: [...], totalElements, totalPages }
```

```console
$ curl -s 'localhost:8080/products?page=0&size=20'   # the first page
```

The first page number is used everywhere: the request, the `page` field and next/previous page numbers in the body, and the `Link` header. On a 1-based route `page=0` is a 400; on a 0-based one `page=-1` is.

How a style is chosen:

- Only **declared query parameters** count (on the operation or its path item). A parameter you send but the spec does not declare is ignored.
- Several index parameters on one route: **cursor beats offset beats page**.
- `page`, `offset` and the size parameter must be **numeric** (`integer` or `number`) in the spec. A `string` page, offset or size is not recognised, and a route with only those is not paged.
- A **cursor** may be a `string` (that is what real APIs do), or any other primitive type. But a string with `format: date`, `date-time` or `time` is never a cursor: an `after=2024-01-01` filter is not paging.
- Only `GET` (and the `HEAD` that follows it) is paged.

## Page size, bounds and errors

| Setting      | Value                                                                             |
| ------------ | --------------------------------------------------------------------------------- |
| Default size | the parameter's `default` in the spec, else **20**                                |
| Largest size | the parameter's `maximum`, else **100**; asking for more is clamped, not an error |
| Smallest     | the parameter's `minimum`, else 1; asking for less is a 400                       |

**Heavy records shrink the page.** One response body holds at most **10 000 values** (every object, array, string, number counts once), the same cap every generated body has. Kaanal works out the most values one record of the route can take (arrays at their longest) and, when `size × that` would pass the cap, serves fewer records per page, never fewer than one. So `?limit=10000` on a route whose records take up to 102 values gets pages of 98. The shrunk size is the page size everywhere: the `limit`/`size` field, `X-Total-Count` stays the total, and the `next` link and field continue exactly where the page ended, so walking still sees every record once.

A bad paging value is a **400**, never a silent fix-up:

```console
$ curl -s 'localhost:8080/pets?limit=0'
{"error":"invalid query parameter","param":"limit","detail":"must be at least 1"}
```

`param` is the parameter as the spec spells it. The 400 cases: a size, page or offset that is not a plain non-negative whole number (no sign, no spaces, digits only), or one above 4294967295 (`exceeds the maximum supported value`), a size below the minimum, a page below the first (`page=0` on a 1-based route), and a cursor Kaanal did not issue. If a parameter is repeated, the first value counts. A page or offset past the end is **not** an error: it is a `200` with no records.

A cursor is `base64url("k1:" + offset)` with no padding. Treat it as opaque; only the exact token Kaanal gave you works, a hand-edited one is a 400.

## The dataset is stable

Every paged route has a fixed number of records, **seeded from the route** (so it is the same on every run and every machine, see [determinism](/docs/guides/mock/endpoints/)). The numbers:

- **Total**: between 23 and 137 records, picked per route. This is not a round number on purpose, so the last page is partial.
- `minItems` / `maxItems` on the paged array in your spec bound the total. If `minItems` is larger than `maxItems`, `minItems` wins (as it does for any array Kaanal generates).
- The total never exceeds **10 000**.
- **Record `i` is the same record** whatever the page size, style or position you reach it by. Changing `limit` or switching from `page` to `cursor` never changes a record's content.
- The `total:` key on the route's [`endpoints:`](/docs/guides/mock/endpoints/) entry sets the total yourself, 0 to 10 000. `total: 0` gives the empty state.

```yaml
endpoints:
  "GET /pets": { total: 250 }
```

## Where the records go (the envelope)

Kaanal looks at the response schema of the route:

- **An array**: the body is the page; the totals travel in headers.
- **An object with an array property**: that array is the page, and the other fields are filled to agree with it. The array is the property named `items`, `data`, `results`, `content` or `records`; if there is none of those, the **first array property in key order** (keys are sorted alphabetically, not in the order you wrote them in the spec).
- An object with no array property is not paged.

The sibling fields of an object envelope are matched by name (case, `_` and `-` ignored) **and** type:

| Field                                                                                           | Type            | Gets                                                |
| ----------------------------------------------------------------------------------------------- | --------------- | --------------------------------------------------- |
| `total`, `totalCount`, `totalItems`, `totalElements`, `count`                                   | integer         | the total                                           |
| `page`, `pageNumber`                                                                            | integer         | the current page number (from 0 on a 0-based route) |
| `limit`, `size`, `pageSize`, `perPage`                                                          | integer         | the page size                                       |
| `offset`                                                                                        | integer         | the offset                                          |
| `totalPages`, `pageCount`, `pages`                                                              | integer         | pages in all (0 when there are no records)          |
| `hasMore`, `hasNext`, `hasNextPage`                                                             | boolean         | whether a later page exists                         |
| `nextCursor`, `next`, `nextPageToken`, `nextPage`                                               | string / number | the next page (below); `null` on the last page      |
| `prev`, `previous`, `prevCursor`, `previousCursor`, `prevPage`, `previousPage`, `prevPageToken` | string / number | the same for the previous page; `null` on the first |

A next/previous field that is a **number** is that page's number. A **string** is, on a cursor route, the cursor token to send back; on a page or offset route, the page's **relative URL** (path and query), the same one the `Link` header carries. That is the Django REST framework shape:

```console
$ curl -s 'localhost:8080/pets?limit=10'
{"count":87,"next":"/pets?limit=10&offset=10","previous":null,"results":[...]}
$ curl -s 'localhost:8080/pets?limit=10&offset=10'
```

Anything else (and a known name of a different type) is generated as it would be without paging. `preview` is not a "previous" field.

```console
$ curl -s 'localhost:8080/orders?page=3&pageSize=5'
{"data":[...5 orders...],"total":64,"page":3,"pageSize":5,"totalPages":13,"hasMore":true}
```

With a string `nextCursor`, follow it to walk the list:

```console
$ curl -s 'localhost:8080/events?limit=3'
{"items":[...],"nextCursor":"azE6Mw"}
$ curl -s 'localhost:8080/events?limit=3&cursor=azE6Mw'
```

## Headers

A paged response carries:

- `X-Total-Count`: the total number of records.
- `Link` ([RFC 8288](https://www.rfc-editor.org/rfc/rfc8288)): relative URLs with the index parameter replaced and your other query parameters kept. Page and offset styles give `first`, `prev` (not on the first page), `next` (not on the last) and `last`; a page past the end links back to real pages; an empty list has only `first`. The cursor style gives only `next`, and none on the last page.

Both are listed in `Access-Control-Expose-Headers`, so a browser app on another origin can read them.

## What is not paged

- A route whose operation declares no recognised paging parameter, or whose response has no list.
- A route with `override:` or `collection:` (a fixed or remembered response; see [collections](/docs/guides/mock/collections/)) and the built-in responders such as the auth node's login. `total:` on them is refused, or warned about.
- A response that is not a normal success. A [pinned](/docs/guides/break/pins/) `empty` answers its empty form and a pinned `error` its error, both with no paging headers. Chaos errors and `reset`/`malformed` wire failures carry none either. A pinned `success` or `slow` still pages. The 400 for a bad query is not a page and has no headers.
- A `HEAD` follows its `GET`: same status and headers, no body. A `"HEAD /pets"` entry of its own that sets no `total:` uses the `GET` entry's.

## Warnings

`kaanal validate` and `kaanal up` print a line for a paging situation worth a look:

- A paged object response with several array properties: `response has 3 array properties; paging uses 'alpha' (the first in key order)`, or `(a list name wins over key order)`.
- `total:` on a route that is not paged.
- `minItems` larger than `maxItems` somewhere in the spec: the arrays get `minItems` items.

## Parameter schemas and $ref

A query parameter's `schema` may be a `$ref` to a component; Kaanal resolves it before reading `type`, `minimum`, `maximum` and `default`. A `schema` `$ref` that cannot be resolved (dangling, or pointing to another file) makes that one parameter be **skipped**, never an error: the route is then simply not paged by it.

A parameter **entry** may itself be a `$ref` (`- $ref: '#/components/parameters/Limit'`). One that cannot be resolved (dangling, or pointing to another file) is skipped too, on any operation (`GET`, `POST`, ...): the spec still loads, and the other parameters still page the route.

Only `in: query` parameters are read for paging. The schemas of header, path and cookie parameters are never resolved here.

> Two regressions worth knowing about, both in the paging work: specs that loaded before it failed to load. The first implementation (commit `fcd529e`) resolved every parameter's schema `$ref` up front, so a header, path or cookie parameter with an external or dangling schema `$ref` failed the **whole spec**; fixed in `1cb501f` by checking `in: query` first and resolving leniently (tests `non_query_params_with_unresolvable_schema_refs_still_load`, `query_param_with_dangling_schema_ref_is_skipped`). The parameter entries themselves were still resolved strictly, so an unresolvable entry `$ref` on any operation failed it the same way; fixed by skipping that entry (test `unresolvable_parameter_entries_are_skipped_not_fatal`).

## Known limits

- The two-array warning also fires for a route that `override` or `collection:` keeps unpaged, because the warning is raised from the spec, before the route's kind is known.
- A `boolean` query parameter named like a cursor (`cursor: boolean`) still counts as a cursor. Cursor parameters may be any primitive type except the date formats above.
- An `integer` cursor parameter (`after: { type: integer }`) also counts as a cursor, but Kaanal's cursors are base64 tokens (`azE6MjA`), so a client that validates the parameter as an integer cannot send them back. Declare such a cursor as a `string`.
- The value budget (how many values the generator will create) is spent **per record**, so record `i` is identical on every page; the page is then shrunk so the records together stay within one body's 10 000 values (see [heavy records](#page-size-bounds-and-errors)). A single record can still use a full budget of its own.
- `collection:` routes are not paged: a collection answers with every item (see [collections](/docs/guides/mock/collections/)).
- No filtering or sorting: other query parameters are kept in `Link` URLs but do not change which records you get.
- With several array properties and no list-named one, the first in **key order** is paged, not the first you wrote.

## Try it

```console
$ curl -si 'localhost:8080/pets?limit=5' | head -4        # first page and headers
$ curl -s 'localhost:8080/pets?limit=100' | wc -c         # clamped to the spec maximum
$ curl -s 'localhost:8080/pets?limit=5&offset=5'         # the second page of 5
$ curl -s 'localhost:8080/pets?offset=100000'             # past the end: []
$ curl -s 'localhost:8080/pets?limit=abc'                 # 400 with the error body
```
