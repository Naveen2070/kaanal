---
title: "Data packs and the dictionary"
sidebar:
  order: 7
---

Kaanal decides what a generated value looks like from two small YAML files you can edit:

- a **locale pack** (`locales/<tag>.yaml`): the names, places, phone numbers, currency and country of one locale;
- a **dictionary** (`data.yaml`): which generator fills which field name.

Both are optional. Without them you get the seven built-in packs (`en-IN`, `en-US`, `en-GB`, `de-DE`, `fr-FR`, `es-ES`, `pt-BR`) and the built-in dictionary. The basics (setting `locale:`, the precedence order, what changed) are in the [user guide](/docs/guides/mock/requests-and-data/#data-and-locales); this page is for writing the files.

## Editor autocomplete

```bash
kaanal schema                # kaanal.yaml (the default)
kaanal schema dictionary     # data.yaml
kaanal schema pack           # locales/<tag>.yaml
```

Each prints a JSON Schema (draft 2020-12). Save it and point your editor's YAML plugin at it. The schemas are generated from the same types the loader uses, so they cannot drift from what `kaanal validate` accepts.

## Writing a locale pack

Create `locales/<tag>.yaml` next to `kaanal.yaml`. A complete pack:

```yaml
# locales/de-DE.yaml (synthetic data, authored for this project)
version: 1
tag: de-DE
name_order: given-family # or family-given
country: Deutschland
currency: { code: EUR, symbol: "€" }
phone: { prefix: "+49", formats: ["{3} {7}", "{4} {6}"] }
address: "{street} {number}, {postcode} {city}"
first_names:
  - { name: Jürgen, latin: Juergen }
  - Anna
  - Lukas
last_names: [Müller, Schmidt, Weber]
streets: [Hauptstraße, Bahnhofstraße, Gartenweg]
places:
  - { city: Berlin, state: Berlin, postcode: "10{3}" }
  - { city: Hamburg, state: Hamburg, postcode: "20{3}" }
  - { city: Leipzig, state: Sachsen, postcode: "04{3}" }
```

`de-DE` is built in, so this file replaces it; use a new tag (e.g. `ja-JP`) to add one, then set `locale: <tag>` in `kaanal.yaml` (or on a node). Built-in packs are synthetic (not from real registries; `es-ES`/`pt-BR` phones are test numbers only). Restart to pick up edits.

### Fields

| Key                         | Required | What it is                                                                                                                                                                                                                                            |
| --------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `version`                   | no       | Pack format version; only `1` exists (default `1`).                                                                                                                                                                                                   |
| `tag`                       | yes      | The locale tag. Must equal the file name without `.yaml`, exactly (case-sensitive).                                                                                                                                                                   |
| `name_order`                | yes      | `given-family` (`Anna Schmidt`) or `family-given` (`Schmidt Anna`), for full names.                                                                                                                                                                   |
| `country`                   | yes      | The name `country` fields get.                                                                                                                                                                                                                        |
| `currency`                  | yes      | `{ code, symbol }`; `code` is what `currency` fields get.                                                                                                                                                                                             |
| `phone`                     | yes      | `prefix` plus a non-empty list of `formats`. A number is the prefix, a space, then one format.                                                                                                                                                        |
| `address`                   | yes      | Template over `{number}`, `{street}`, `{city}`, `{state}`, `{postcode}`. Any other placeholder is an error.                                                                                                                                           |
| `first_names`, `last_names` | yes      | Non-empty lists. Each entry is a plain string or `{ name, latin }`.                                                                                                                                                                                   |
| `streets`                   | yes      | Non-empty list of street names for `{street}`.                                                                                                                                                                                                        |
| `places`                    | yes      | Non-empty list of `{ city, state?, postcode }` rows. A city, state and postcode of one object always come from one row.                                                                                                                               |
| `geo`                       | no       | `{ lat: [min, max], lon: [min, max] }` in degrees, the range `latitude`/`longitude` fields draw from. Without it, the whole globe (-90 to 90, -180 to 180). Each min must not exceed its max; `lat` stays within -90 to 90, `lon` within -180 to 180. |

Unknown keys are errors, so a typo shows up at once with its line and column.

### Patterns

In `phone.formats` and in each place's `postcode`, `{n}` means n seeded digits (n from 1 to 20): `"10{3}"` is `10` followed by three digits. Nothing else in braces is allowed there. `{number}` in the address template is a seeded number from 1 to 999. `{street}` is drawn from `streets`, and is the same street a `street` field of the object gets.

### Names, scripts and emails

Write names in their native script. Emails and usernames are always ASCII at `example.com`, so each name also needs an ASCII form, found in this order:

1. its `latin:` value, if you gave one (`{ name: 田中, latin: tanaka }`; it must be ASCII);
2. else the name lower-cased with diacritics folded (`José` becomes `jose`, `Straße` becomes `strasse`);
3. else, for a name with nothing foldable (`鈴木`), `user` plus a number.

Spaces, apostrophes and hyphens are dropped from the ASCII form. Length limits in a schema (`maxLength`, `minLength`) count characters, not bytes, so non-Latin packs fit correctly.

### States

`state` is optional per place. If your `address` template uses `{state}`, every place needs one (otherwise the pack fails to load, naming the place). A pack without states is fine; a `state` field then gets the city instead.

### Replacing a built-in

A project pack with the same tag as a built-in replaces it wholesale; there is no partial merge. The dictionary is where you adjust individual fields.

### Where packs are found

Kaanal lists `locales/` next to the config. The rules are the same as for scenario files:

| Rule                 | Detail                                                                                                                                                                                                         |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What counts as pack  | Only regular files named exactly `*.yaml` are packs. Subdirectories, dotfiles and `*~` editor files are ignored silently.                                                                                      |
| Skipped with warning | Symlinks are skipped with a warning ("symlinks are not followed"). So are `.yml` and `.YAML` files ("pack files must end in .yaml").                                                                           |
| Size limits          | At most 64 pack files, each at most 1 MiB. More files is an error at the `locales/` directory.                                                                                                                 |
| File name            | A file name must be a locale tag (such as `de-DE.yaml`) and equal the `tag:` inside. A mismatch is an error at the `tag:` value.                                                                               |
| Case                 | Two packs whose tags differ only in case (`de-de` next to `de-DE`) are an error naming both. Tags are looked up exactly; an unknown `locale:` that matches one in a different case gets `did you mean de-DE?`. |
| Loading              | **Every file in `locales/` is loaded and checked at startup**, not only the ones a `locale:` names, so a broken pack fails `kaanal validate` and `kaanal up` even if nothing uses it.                          |

A tag is never turned into a path: `locale:` must match `^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$` (`en`, `en-IN`, `zh-Hant-TW`) and is looked up among the loaded packs only. `../x` and `en_IN` are rejected.

## Writing a dictionary

`data.yaml` next to `kaanal.yaml` (or `data:` path) adds a dictionary layer **before** schema `format` (the built-in dictionary runs **after** `format`; see [Precedence and fit](#precedence-and-fit)).

- Any matching key in your file beats built-in entries (even `"*id"` over built-in `orderId`).
- If your entry doesn't fit the field, `format` → built-in dictionary → defaults decide. Unmatched fields keep built-in behaviour.

Unlike packs in `locales/`, a `data.yaml` (or `data:` file) that is a symlink is followed.

```yaml
version: 1
fields:
  email: { gen: email }
  price: { gen: money, min: 99, max: 4999 }
  status: { gen: pick, from: [paid, refunded, pending] }
  sku: { gen: template, value: "SKU-{digits:6}" }
  "*id": [{ gen: uuid }, { gen: integer, min: 1, max: 99999 }]
```

An empty `data.yaml` is fine.

### Keys

| Rule          | Detail                                                                                                                                                                                                                                                                                                                                                                 |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Normalisation | Names are normalised: lower-case, with `_` and `-` removed. `userEmail`, `user_email` and `User-Email` share one key.                                                                                                                                                                                                                                                  |
| Suffix keys   | A key starting with `*` is a **suffix** key. It matches the last **whole words** of a field name, where words are split at `_`, `-` and case changes. `*id` matches `userId`, `user_id` and `ID`, but never `valid`, `paid` or `android`. An all-lower-case compound such as `unitprice` is one word, so `*price` does not match it (`unitPrice` and `unit_price` do). |
| Precedence    | An exact key beats a suffix key. Among suffix keys the longer one beats the shorter one.                                                                                                                                                                                                                                                                               |
| Duplicates    | Two keys in your file that normalise alike (`user_email` and `userEmail`) are a warning; the later one wins.                                                                                                                                                                                                                                                           |
| Quoting       | Quote suffix keys in YAML: `"*id"`.                                                                                                                                                                                                                                                                                                                                    |

### Generators

An entry is `{ gen: <name>, ...parameters }`. The set is closed: a dictionary cannot run code, and an unknown generator or parameter is an error naming the valid ones.

| `gen`                                  | Output                                                                                           | Parameters                                                                                           |
| -------------------------------------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `first_name`, `last_name`, `full_name` | The object's person from the pack (full name follows `name_order`)                               |                                                                                                      |
| `email`, `username`                    | ASCII, built from the same person; emails at `example.com`                                       |                                                                                                      |
| `phone`                                | Pack prefix and one of its formats                                                               |                                                                                                      |
| `city`, `state`, `postcode`, `address` | The object's place; they always agree with each other                                            |                                                                                                      |
| `country`, `currency`, `street`        | From the pack                                                                                    |                                                                                                      |
| `url`, `uuid`                          | `https://example.com/...`, a v4 UUID                                                             |                                                                                                      |
| `timestamp`, `date`                    | RFC 3339 timestamp or date before 2026-01-01; two in three fall in the last third of the window  | `days` (window, default 365)                                                                         |
| `integer`                              | A whole number                                                                                   | `min`, `max`, `dist: uniform` (default) or `bands`                                                   |
| `money`, `latitude`, `longitude`       | A number with 2 (money) or 6 decimals; coordinates default to the pack's `geo` range             | `min`, `max`; money also `dist: decade` (default) or `uniform`, `style: plain` (default) or `retail` |
| `text`                                 | Lorem text                                                                                       | `kind: sentence` (default) or `paragraph`                                                            |
| `pick`                                 | One of your values, by seed                                                                      | `from` (non-empty list), `weights`                                                                   |
| `template`                             | A string with `{digits:n}` and `{letters:n}` (n from 1 to 20) filled by seed: `"SKU-{digits:6}"` | `value`                                                                                              |
| `static`                               | Your value, as is                                                                                | `value`                                                                                              |

`min` must not exceed `max`. Both are optional; a lone bound keeps the default range's other end when it can.

### Realistic spread

Real data is lumpy, so the built-in dictionary is too (all of it can be changed in your `data.yaml`):

| Topic                        | Behaviour                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Prices                       | Skewed cheap (`dist: decade`) and ending like shop prices (`style: retail`: `.99` 50%, `.00` 25%, else `.95`/`.49` on `price`/`*price`/`cost`). Retail endings are dropped if they violate bounds, integer prices are unchanged, ranges `<= 0` draw uniformly, and `dist: uniform, style: plain` turns both off.                                                                                                                                                                                                                                                                                          |
| Weighted choices (`weights`) | Maps value text (`paid`, `true`, `4`) to positive weights (unlisted = `1`, at least one `> 0`). Built-ins favour happy-path statuses (`status`, `*status`, `order_status`, `payment_status`: `8` active/paid/delivered, `3` pending/shipped, `1` cancelled/failed), `true` flags (`active`, `enabled`, `verified`: 4 in 5), positive `rating` and `stars`, and small quantities (`quantity`: mostly `1–3`). Weights also apply to an `enum` on a field with that name (weighted, never limited). An entry of your own for the same name, without `weights`, turns the built-in weights off for that name. |
| Timestamps                   | Lean recent: two thirds of values fall in the most recent third of the window (`days`, default 365, back from 2026-01-01).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Ages (`age`)                 | Come from four bands over 18 to 80 weighted 25/30/25/20, so most people are between 33 and 64; the birth date then follows the age (see Records that add up).                                                                                                                                                                                                                                                                                                                                                                                                                                             |

### An entry can be a list

```yaml
"*id": [{ gen: uuid }, { gen: integer, min: 1, max: 99999 }]
```

The generators are tried in order and **the first whose output fits the field's schema type applies**. Here a string `userId` gets a UUID and an integer `userId` gets a number.

### Time chains

Timestamps on one object stay ordered: `createdAt → updatedAt/modifiedAt → deletedAt`, `startAt/startDate → endAt/endDate`, `orderedAt → paidAt → shippedAt → deliveredAt`, and `issuedAt → dueAt`.

Each later field draws between the previous one and `2026-01-01` (adding `updatedAt` never shifts `createdAt`). A member with an `example`, `enum`, `const` or a `static`/`pick` entry of yours is never changed, and later members follow it. Mixed date/date-time chains are left alone.

Add your own chains, or switch built-in ones off, in `data.yaml`:

```yaml
version: 1
chains:
  - [packed_at, sealed_at]   # earliest first, two or more names
chains_off: [created_at]      # drops every built-in chain that contains this name
```

### Records that add up

Related fields on one object stay mathematically and semantically consistent:

| Group         | Rule                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Orders        | Line items (price × quantity) set exact-cent `lineTotal`, `subtotal`, `tax`, `shipping`, `discount` (<= subtotal), and `total` = subtotal + tax + shipping - discount. `tax` is a fraction of the subtotal. Sums are exact cents (never `59.970000000000006`); all-integer schemas stay integers. A bare `total` only sums when typed `number` or alongside `subtotal`/`tax`/`shipping`/`discount` (otherwise an integer `total` is a list count). A plain list such as `{ items: [{name, price}], total: 3 }` is never rewritten. |
| Demographics  | `age` and `dateOfBirth`/`dob`/`birthday` match to the year (without `age`, birth dates are 18–80 years before 2026-01-01). `locale` and `language` match the node's locale (`en-IN` and `en`).                                                                                                                                                                                                                                                                                                                                     |
| Pinned fields | A field you pinned with an `example`, `enum`, `const` or `static`/`pick` entry is never rewritten.                                                                                                                                                                                                                                                                                                                                                                                                                                 |

### Precedence and fit

Kaanal picks a value in this order, first match wins:

1. the schema's `example`, then `examples` (response-level `example`/`examples` replace the schema's own);
2. `const`, then `enum`;
3. `oneOf`/`anyOf` (the seed picks a branch, which then goes through this list);
4. **your `data.yaml`**;
5. the schema's `format`;
6. the built-in dictionary;
7. defaults from the schema's type and bounds.

Specific beats general: a `format: email` field beats a built-in guess by name, and your dictionary beats a `format`.

The schema's type and bounds always beat a dictionary entry. An entry applies only if its output can fit the field:

- `money`, `integer`, `latitude` and `longitude` are clamped to `minimum`/`maximum`;
- `text`, names, places and other generated strings are trimmed to `maxLength` (in characters) and padded to `minLength`;
- string `pick`, `static` and `template` values are **never** trimmed or padded: they are sent exactly as written;
- `pick` and `static` values that do not fit the type or the numeric bounds are not used;
- a string generator on an integer field (or the reverse) does not fit.

Unfitting entries fall through to the next layer. A misfit in **your** `data.yaml` warns on `kaanal validate` and `kaanal up`, naming the endpoint and field (`GET /users .age: project dictionary entry for age does not fit...`). Built-in misfits skip silently, because they were only a guess from the field name.

Your own `static`, `pick`, and `template` strings are sent verbatim even if they fall outside `minLength`/`maxLength` (with a startup warning such as `GET /items .sku: project dictionary value for sku can fall outside its minLength/maxLength; it is sent as written`); built-in ones skip silently and fit the length. A template's length is known because its placeholders have fixed widths.

`pick`, `static` and `template` values are your own data and are not checked for being synthetic; the built-in packs and dictionary are, and so should anything you contribute (below).

## Contributing a pack

Kaanal's built-in packs are open to contributions. The rule (FR-09):

> **Synthetic data only, authored for Kaanal.** Do not copy a census list, a registry, a scraped site, someone's dataset or any real person's details, even if it is freely licensed. Write generic given names and surnames yourself, list real city names (they are public geography) and invent postcode prefixes of the right shape. Mark the file as synthetic in a header comment.

Before sending a pack:

- every required block is present and lists are meaningful in size (the built-in packs have about 200 first names, 200 surnames and 60 places);
- no duplicates in a list;
- `latin:` forms are ASCII, and every name that is not Latin script has a way to an email (`latin:` or the `user<n>` fallback);
- postcodes look like the country's;
- `kaanal validate` is clean.

## Errors you may see

Every error carries `file:line:col` where YAML can place it.

| Message (shortened)                                              | Fix                                                                                                  |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `unknown locale x; available: ...`                               | Add `locales/x.yaml` or use a listed tag. The message includes `did you mean` for a case difference. |
| `tag: x must match the file name y`                              | Rename the file or fix `tag:`.                                                                       |
| `differs from the pack 'x' only in case`                         | Use one spelling.                                                                                    |
| `places: 'X' has no state, but address uses {state}`             | Give that place a state or drop `{state}` from the template.                                         |
| `latin: ... must be ASCII`                                       | Use letters `a-z`, digits only.                                                                      |
| `address: ... unknown placeholder`                               | Use `{number}`, `{street}`, `{city}`, `{state}`, `{postcode}`.                                       |
| `unknown variant x, expected one of ...` (dictionary gen)        | Use a generator from the table above.                                                                |
| `template '...' has an unclosed brace or an unknown placeholder` | Use `{digits:n}` or `{letters:n}`.                                                                   |
| `... pack files; at most 64`                                     | Remove unused packs.                                                                                 |
| `data: ... is not a file`                                        | `data:` must name a file, not a directory.                                                           |
