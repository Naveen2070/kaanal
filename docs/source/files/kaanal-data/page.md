# `src/page/mod.rs`, `src/page/window.rs`, `src/page/envelope.rs` and `src/page/links.rs`

Paging style detection, window resolution and page generation for an endpoint (design spec: D3 paging).

## Public API

- `Style` — `Page`, `Offset` or `Cursor`.
- `PageSpec` — `style`, `index_param`, `size_param`, `size_default`, `size_min`, `size_max`, `total_min`, `total_max`, `envelope`, `page_base` (0 or 1), `record_cap` (most records a page may hold).
- `PageSpec::detect(&Endpoint) -> Option<PageSpec>` — `None` unless the endpoint is a `GET` that declares a recognised index parameter and whose schema has an envelope.
- `Envelope` (`envelope.rs`) — `Array` or `Object { items }`; `Envelope::detect(&schema) -> Option<Envelope>`.
- `generate_page(schema, &PageSpec, &Window, seed, &Options, path, query) -> Value` (`envelope.rs`) — the body of one page; `path` and `query` are the request's, for string `next`/`prev` URLs.
- `item_seed(seed, i)` — the seed of record `i`: `child_seed(seed, "item", i)` (R4.4).
- `PageSpec::window(query, seed, total_override) -> Result<Window, PageError>` (`window.rs`) — resolves `Window { total, offset, size }`.
- `PageError::Invalid { param, detail }` with `body()`, the `400` JSON body.
- `encode_cursor(offset)` / `decode_cursor(&str)` — `base64url("k1:" + offset)` without padding.
- `Window::headers(&PageSpec, path, query) -> Vec<(&'static str, String)>` (`links.rs`) — `x-total-count` and the RFC 8288 `link` header.

## Rules

- Names are normalised (lower-case, `_` and `-` removed) before matching.
  - index: `page`, `pagenumber`; offset: `offset`, `skip`; cursor: `cursor`, `after`, `pagetoken`, `nextpagetoken`, `startingafter`.
  - size: `limit`, `size`, `pagesize`, `perpage`, `count`, `maxresults`.
- Page/offset index and size parameters must be numeric (`QueryParam::numeric`); a cursor may be a string.
- When several index parameters are declared: cursor, then offset, then page.
- Size: default is the spec `default` else 20; max is the spec `maximum` else 100; min is the spec `minimum` else 1.
- Total bounds: 23..=137, or the paged array's `minItems`/`maxItems` (the response itself, or the envelope's array property); a `minItems` above `maxItems` wins, as in the generator; hard cap 10 000.
- Page base: 0 when the page parameter declares `minimum: 0`, or `default: 0` with no `minimum` above 0 (Spring); else 1. Offset and cursor styles: 1.
- Record cap (`Envelope::record_cap`): `(MAX_VALUES - envelope's own worst case) / record's worst case`, at least 1. The worst case is `generate::value_bound`: the generator's walk with every array at its longest draw, the dearest `oneOf`/`anyOf` branch and three `additionalProperties` keys, capped at `MAX_VALUES`.

## Window rules (`window.rs`)

- Size bounds are normalised first: min >= 1, max >= min, default clamped into both. A size below min (or 0, or non-integer) is `400`; above max is clamped; then the size is shrunk to `record_cap` (at least 1), before a page number becomes an offset, so every page of a route has the same size. A number is ASCII digits only (no sign, no whitespace); digits past `u32::MAX` are a `400` saying "exceeds the maximum supported value". A repeated key takes its first value.
- `Page`: offset = (page - page_base) * size, saturating; a page below `page_base` (page 0 on a 1-based route) is `400`. `Offset`: the value. `Cursor`: decoded; a forged or non-canonical cursor (stray trailing bits, leading zeros: re-encoding must give the same token) is `400`. A window past the end is not an error.
- Total: seeded by `child_seed(seed, "total", 0)` within `total_min..=total_max`, or `total_override`; both capped at 10 000.

## Envelope rules (`envelope.rs`)

- `allOf` is merged first; "object" and "array" mean what the generator treats them as (a typeless schema with `properties` is an object).
- Array schema: `Envelope::Array`, the body is the slice. Object: the first array property named `items`, `data`, `results`, `content` or `records`, else the first array property, both in key order (maps are sorted: no `preserve_order`, so not the order the spec writes them). Several array properties: one startup warning (`warn.rs`). No array property: not paged.
- A page holds records `[offset, min(offset + size, total))`, each `generate(items, None, item_seed(seed, i), opts)` with its own value budget, so record `i` is the same on any page, size or style; the window's size is already within `record_cap`, so the page stays within `MAX_VALUES` (one record alone may use a whole budget). Past the end: empty, metadata still right.
- Object envelope: generated as today from the schema with its paged array replaced by `{type: array, maxItems: 0}` (a large `minItems` costs nothing), then the array and the meta siblings (normalised name and JSON type must both match) are overwritten:
  - integer: `total`/`totalcount`/`totalitems`/`totalelements`/`count` = total (`count` is the total, as in DRF; 2026-10-26); `page`/`pagenumber` = offset / size + page_base (saturating); `limit`/`size`/`pagesize`/`perpage` = size; `offset`; `totalpages`/`pagecount`/`pages` = ceil(total / size), 0 when total is 0.
  - boolean: `hasmore`/`hasnext`/`hasnextpage` = offset + size < total.
  - `nextcursor`/`next`/`nextpagetoken`/`nextpage` and `prev`/`previous`/`prevcursor`/`previouscursor`/`prevpage`/`previouspage`/`prevpagetoken` (exactly; `preview` is not one): a string is `encode_cursor` of the next/previous offset in the cursor style, and that page's relative URL (`PageSpec::page_url`, the same the `link` header carries) in the page and offset styles; a number is that page number (from `page_base`); `null` when there is none.
  - Any other sibling, or a known name of another type, is left as generated.

## Link rules (`links.rs`)

- Links are relative (`PageSpec::page_url`): the request path plus the query with the index parameter replaced (appended last); other pairs keep their order and are percent-encoded (unreserved characters kept).
- Page: `first` page=page_base, `last` page=ceil(total/size) - 1 + page_base (page_base when empty), `prev` only when offset > 0, `next` only when offset + size < total. Offset: same with offsets (`last` = (pages - 1) * size).
- Past the end: `first`, `prev` (clamped to the last real page), `last`, no `next`. Empty: `first` only.
- Cursor: only `rel="next"` (an `encode_cursor` token), none on the last page; no `link` header when there is no relation.

## Tests

`tests/page/detect.rs`, `tests/page/window.rs`, `tests/page/envelope.rs`, `tests/page/links.rs`, `tests/page/walk.rs` (the oracle: walking every page equals records `0..N` generated one by one).
