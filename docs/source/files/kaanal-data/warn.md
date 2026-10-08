# `crates/kaanal-data/src/warn.rs`

Warnings for schema keywords the generator does not honour.

## What it does

The generator ignores some JSON Schema keywords (`not`, `pattern`, `patternProperties`, `uniqueItems`, `multipleOf`, `discriminator`) — generated values may violate them. Rather than staying silent, every ignored keyword becomes a one-line warning that `kaanal validate` and `kaanal up` print (rule R4.6).

## Items

| Item                                              | Kind  | What it does                                                                                                                                                                                                                                                                |
| ------------------------------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `IGNORED`                                         | const | The six keywords above. **Keep in sync with the generator**: if the generator learns one, remove it here.                                                                                                                                                                   |
| `unsupported_keywords(schema, at) -> Vec<String>` | fn    | One warning line per ignored keyword found. `at` names the endpoint (`"GET /users"`); nested properties are appended (`"GET /users .code"`). Property _names_ that happen to equal a keyword (a field called `pattern`) are never flagged — only actual keyword positions.  |
| `walk(s, at, out)`                                | fn    | Recursive walk: flags keywords on the current schema object, flags `maxLength` too small to hold a `format` value (uuid 36, date-time 20, date 10 — the format wins, rule R4.5), then recurses into `properties`, `items`, `additionalProperties`, `allOf`/`oneOf`/`anyOf`. |
| `ApiModel::warnings()`                            | impl  | Runs `unsupported_keywords` over every endpoint's inlined response schema, collecting all warnings. Called by the server's `check`.                                                                                                                                         |

## Tests

[../tests.md](../../tests.md#crateskaanal-datatestswarningsrs) — keyword-vs-property-name distinction, combinator walk, maxLength/format conflict, per-endpoint collection.
