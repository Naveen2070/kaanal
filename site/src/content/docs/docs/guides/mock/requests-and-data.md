---
title: "Requests and data"
sidebar:
  order: 1
---

## What a request looks like

```bash
curl -i http://localhost:8080/users/42
```

```http
HTTP/1.1 200 OK
x-kaanal-trace: nginx.proc=1.0ms,net=1.0ms,app.proc=5.0ms,net=1.0ms,db.read=4.2ms
x-request-id: r04f2-0042
access-control-allow-origin: *

{ "id": 42, "firstName": "Priya", "email": "priya.iyer42@example.com", ... }
```

- The **body** is generated from your OpenAPI response schema — real-looking names, emails, pincodes — and is identical on every call to the same URL. Path params flow into matching fields (`id: 42`).
- The **timing** is what the trace header adds up to: hops in sequence add; a `parallel` block costs its slowest branch. The client actually waits it.
- A `parallel` step shows its branches in brackets, split by `|`, and a cache lookup shows `hit` or `miss`: `nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms`. A failure ends the header with `node.error=STATUS`. Very long chains end with `,...+N hops`, where N is the number of hops left out.
- When a node is full, a request queues and the trace shows it before the hop it waited for: `...,net=1.0ms,db.wait=98.0ms,db.read=100.0ms`. A node with a `rate_limit` answers `429` with a `Retry-After` header (whole seconds) once its tokens run out.
- `X-Request-Id` is the request's id. Run `kaanal trace r04f2-0042` to see that request hop by hop, or `kaanal logs` for the latest ones; the 1000 most recent are kept. See [Observability](/docs/guides/observe/observability/).
- **CORS** is wide open on mock routes, so any local frontend just works.

## Data and locales

Generated bodies follow your OpenAPI response schemas. Three things shape what the values look like: the **locale** (whose names, places, phones and currency), the **dictionary** (which generator fills which field name) and the schema keywords themselves. Same Kaanal version, config, data files and `seed:` give the same bytes every time.

## Locale

```yaml
version: 1
locale: en-IN              # the project default; en-IN if omitted
nodes:
  app:    { type: service, spec: openapi.yaml }
  us-api: { type: service, spec: us.yaml, locale: en-US }   # this node's routes use en-US
```

- `locale:` is a tag such as `en-IN` or `en-US`. A node's own `locale:` beats the project default; the locale comes from the node that owns the route.
- Seven packs are built in: `en-IN` (names, `+91`, `INR`, `India`), `en-US` (`+1`, `USD`, `United States`), `en-GB` (`+44`, `GBP`), `de-DE` (`+49`, `EUR`, no states), `fr-FR` (`+33`, `EUR`), `es-ES` (`+34`, `EUR`) and `pt-BR` (`+55`, `BRL`, states).
- Any other tag needs a pack: drop `locales/<tag>.yaml` next to `kaanal.yaml`. A pack with a built-in's tag replaces it. [Data packs](/docs/guides/mock/data-packs/) shows how to write one and where Kaanal looks.
- An unknown locale is a startup error that lists the available tags (and says `did you mean` when only the case differs). `kaanal validate` and `kaanal up` check every pack in `locales/`, used or not.

## Dictionary

`data.yaml` next to `kaanal.yaml` (or `data: path/to/file.yaml` at the top level) overrides the built-in field dictionary of about 150 names:

```yaml
version: 1
fields:
  price:  { gen: money, min: 99, max: 4999 }
  status: { gen: pick, from: [paid, refunded, pending] }
  sku:    { gen: template, value: "SKU-{digits:6}" }
  "*id":  [{ gen: uuid }, { gen: integer, min: 1, max: 99999 }]
```

Field names match whatever their style (`userEmail`, `user_email`); a `*suffix` key matches whole trailing words only (`*id` matches `userId`, not `paid`); an entry can be a list, the first generator that fits the field's type applying. All of it, the generator list and the error messages are in [Data packs](/docs/guides/mock/data-packs/#writing-a-dictionary).

Precedence, first match wins:

1. schema `example`, then `examples` (a response-level `example`/`examples` beats the schema's own);
2. `const`, then `enum`;
3. `oneOf`/`anyOf`: the seed picks a branch;
4. your `data.yaml`;
5. `format`;
6. the built-in dictionary;
7. type and bounds defaults.

A string with a `pattern` matches it: a generated value that already matches (an email under `.+@.+`) is kept, otherwise Kaanal samples one from the regex (ASCII only, open repeats up to 8 extra, `minLength`/`maxLength` respected). Your `example`, `enum`, `const` and `data.yaml` `static`/`pick` values are never replaced. A pattern Kaanal cannot sample (lookaround, back-references, a huge repeat) or cannot fit within the length bounds is a startup warning, and the value may not match it.

The schema's type and bounds always win over a dictionary entry: numbers are clamped to `minimum`/`maximum`, generated strings fitted to `maxLength` and `minLength` (in characters). An entry that cannot fit its field is skipped. A misfit in your `data.yaml` is a warning from `kaanal validate` and `kaanal up` that names the endpoint and field; a misfit built-in entry is skipped silently. Your own `pick`, `static` and `template` strings are never trimmed or padded: they are sent as written, and one that can fall outside the field's `minLength`/`maxLength` is a warning naming the field.

## Schema keywords

| Keyword                           | What Kaanal does                                                                                                                                                                                                      |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `example`, `examples`             | Used verbatim; `examples` (a list, or an OpenAPI map of `{ value }`) is picked from by seed.                                                                                                                          |
| `const`                           | The constant.                                                                                                                                                                                                         |
| `format`                          | Adds `hostname`, `ipv4`/`ipv6` (documentation ranges only: `198.51.100.x`, `203.0.113.x`, `2001:db8::`), `byte`, `password`, `duration` and `int32`/`int64` bounds to `email`, `uuid`, `date-time`, `date` and `uri`. |
| `oneOf`, `anyOf`                  | The seed picks one branch, uniformly. Adding or reordering other properties never changes the pick.                                                                                                                   |
| `nullable`, `type: [x, "null"]`   | Read; a real value is always generated, never `null`.                                                                                                                                                                 |
| `additionalProperties` (a schema) | 0 to 3 extra seeded keys (`key1234`); none for `false`, `true` or an absent keyword.                                                                                                                                  |
| `discriminator`                   | The chosen branch gets its tag value; see below.                                                                                                                                                                      |

### Discriminators

With `oneOf`/`anyOf` and a `discriminator`, the picked branch's tag property is set so the body matches exactly one branch:

- With a `mapping`, the tag is a mapping key that names that branch (by `#/components/schemas/Dog` or just `Dog`). A branch named by several keys gets one of them, by seed.
- Without a `mapping`, the tag is a single-value `enum` or a `const` on the discriminator property of the branch. Any non-empty `enum` counts.
- There is **no implicit tag** from the component name: with neither a mapping nor an `enum`/`const`, the tag is generated like any other field.
- An object-level `example` on a branch beats the tag.

`kaanal validate` and `kaanal up` warn about the cases that cannot work:

- `discriminator 'petType' has no mapping, enum or const for oneOf branch 1; its tag is generated normally`
- `discriminator mapping 'cat' names no oneOf/anyOf branch; it is never generated`
- `discriminator has no propertyName and is ignored`

It also samples each endpoint's generated response (5 bodies) and checks them against the response schema. A miss is a warning that names the endpoint and the path, for example `GET /orders: 3 of 5 generated samples violate the response schema: /items/*/total: maximum`. It is Kaanal's bug or a contradiction in the spec (an `example` outside its own `maximum`); the mock still starts. `kaanal up` prints these lines a moment after it has started serving, so a big spec never slows startup.

## Behaviour changes in this release

Kaanal is pre-1.0, and the same version, config, files and seed give the same data, but data may change between `v0.0.x` releases. If you diff generated bodies or commit them as fixtures, expect these differences:

- All built-in output is new: `en-IN` names, places, phones and addresses come from a larger pack, and other values may draw differently. Fixtures that must never change belong in an `override:` body or a schema `example`.
- `oneOf` and `anyOf` used to always generate the first branch; they now pick a branch by seed.
- `ipv4` values moved to documentation ranges, `ipv6` and `hostname` are new (`2001:db8::...`, `*.example.com`).
- `street` is a street name (it used to be the full address), `status` picks from `active`, `inactive` and `pending` (it was always `active`), and any field ending in `At` or `_at` (`shippedAt`, `expires_at`) is a timestamp.
- Suffix matching is by whole word. An all-lower-case compound such as `unitprice` no longer matches the money suffixes (`unitPrice` and `unit_price` do), and `valid`, `paid` or `android` never match `*id`.
- New keywords change how much of a body's value budget a schema uses (`const`, `additionalProperties`, `oneOf`), so a very large nested schema may reach its cap at a different place.
- `discriminator` is no longer reported as an ignored keyword.
- A field ending in `Total`/`_total` (`orderTotal`, `grandTotal`) or `subtotal` is money now, number and integer alike. A bare `total` stays a count from 0 to 1000, and is a whole number on a `number` field too.
- `street` and the street inside `address` are now the same street for one object.
- Records now agree with themselves (D2): `updatedAt` is never before `createdAt` (and the other timestamp chains), `age` matches `dateOfBirth`, an order's `lineTotal`, `subtotal`, `tax`, `shipping`, `discount` and `total` add up, and a string with a `pattern` matches it. `multipleOf` and `uniqueItems` (scalar items) are honoured. A field you pinned with `example`, `enum`, `const` or a `static`/`pick` entry is never rewritten.
- Values are spread more like production (D2): prices lean cheap and end in `.99`, `.00`, `.95`, status words favour `paid`/`active`/`delivered`, flags such as `active` are mostly true, `rating` leans positive, `quantity` is mostly 1 to 3, timestamps lean recent, `age` has more people between 33 and 64. Every value stays possible. Fixtures that must never change belong in an `override:` body or an `example`.
- `kaanal validate` and `kaanal up` now also generate five bodies per endpoint and check them against the response schema; see Warnings above.
- Built-in packs carry coordinate bounds (`geo:`): `latitude`/`longitude` stay inside India for `en-IN` (unchanged) and inside the continental US for `en-US` (they were India's for every locale); a pack without `geo:` uses the whole globe.
- `empty` pin bodies use the owning node's locale and your `data.yaml` (they always used `en-IN` and the built-ins), and each field draws from its own seed, so the values change.
- `en-IN` phone numbers are `+91 9` and nine digits, which can look like a real Indian mobile number. Use them as test data only; never send messages to them.
