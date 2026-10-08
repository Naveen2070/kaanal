# `crates/kaanal-data/src/api.rs`

OpenAPI 3.0.x → `ApiModel`: the list of endpoints the server will mock.

## What it does

Parses a spec (YAML or JSON — YAML is a superset) into one `Endpoint` per `path × method`, picking the success response and inlining every local `$ref` in its JSON schema, so downstream code never has to resolve references.

Phase one supports **3.0.x only**; 3.1 is rejected with a clear message.

## Types

| Item | Kind | What it does |
|---|---|---|
| `Endpoint` | struct | `method` (uppercase), `path` (OpenAPI template, also valid matchit 0.8 syntax), `status` (chosen 2xx), `schema` (response JSON schema with refs inlined; `None` = no JSON body). |
| `ApiModel` | struct | `endpoints: Vec<Endpoint>`. |
| `ApiError` | struct | Single message string; `Display` passes it through. |
| `MAX_REF_EXPANSIONS` | const | 10 000 — cap on `$ref` expansions per schema (see below). |

## Functions

| Function | What it does |
|---|---|
| `ApiModel::from_openapi(src)` | Parses the document; extracts the version (accepting unquoted `openapi: 3.0`, which YAML reads as a number); rejects non-3.0.x; walks `paths` for the 7 HTTP methods; each path must start with `/`. |
| `pick_response(op, doc, ptr)` | Chooses the **lowest 2xx** response code (404s etc. are ignored; no 2xx → `(200, None)`). Resolves a response-level `$ref`, then digs out `content.application/json.schema` and inlines it. `ptr` is the JSON-pointer of the operation, used in error messages. |
| `inline(v, doc, ptr, stack, budget)` | Recursively replaces every local `$ref` with its target. Two safety rails: `stack` detects **cycles** (recursive schemas → error on first revisit, even through fan-out), and `budget` caps total expansions so an acyclic but exponentially-branching ref chain ("billion laughs") is rejected instead of eating memory. Both errors name the operation pointer. |
| `resolve_ref(r, doc, ptr)` | Local refs (`#/...`) only, resolved as JSON pointers. External refs (`other.yaml#/...`) are rejected. |

## Error behaviour

Every failure names the operation (`/paths/~1users/get`) and says exactly what is wrong: unsupported version, external `$ref`, recursive `$ref`, missing target, too many expansions. Covered by `kaanal-data/tests/api.rs` (see [../tests.md](../../tests.md#crateskaanal-datatestsapirs)).
