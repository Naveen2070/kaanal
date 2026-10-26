# Kaanal user documentation (docs site)

This folder holds the **user-facing documentation** — the content of Kaanal's public docs site. It is written for people _using_ the tool, not changing it. (Contributors: you want [../coding-guide/](../coding-guide/index.md) and [../source/](../source/architecture.md).)

## Site structure

| Page                                 | Audience                      | Contents                                                                     |
| ------------------------------------ | ----------------------------- | ---------------------------------------------------------------------------- |
| [user-guide.md](user-guide.md)       | Everyone                      | Install, `kaanal.yaml`, CLI, the embedded status page                        |
| [control-api.md](control-api.md)     | Test authors, tooling writers | The `/__kaanal/*` HTTP API, request/response reference                       |
| [nodes.md](nodes.md)                 | Everyone                      | Every node kind, its fields, the edge table and presets                      |
| [concepts.md](concepts.md)           | Everyone                      | Topology, call chains, chaos, determinism — what Kaanal actually simulates   |
| [web-ui.md](web-ui.md)               | Users of the status page      | The embedded `/__kaanal/ui` page, feature by feature                         |
| [endpoints.md](endpoints.md)         | Everyone                      | Endpoint chains, error maps, determinism and kaanal endpoints                |
| [auth.md](auth.md)                   | Everyone                      | Login and JWKS from an auth node: config, tokens, keys, limits               |
| [capacity.md](capacity.md)           | Everyone                      | Slots, queues, rate limits, 429s and the `wait` trace piece                  |
| [scenarios.md](scenarios.md)         | Everyone                      | Named chaos scenarios: YAML, stacking, CLI, API, reset, test-runner recipes  |
| [pins.md](pins.md)                   | Frontend developers, testers  | Pinned UI states (loading, empty, error, slow) and `kaanal cycle`            |
| [observability.md](observability.md) | Everyone                      | Request ids, `kaanal logs`, `kaanal trace`, flags, test-runner recipes       |
| [lifecycle.md](lifecycle.md)         | CI authors, test authors      | `up -d`, `down`, stale state, signals, the drain, exit codes, CI recipes     |
| [data-packs.md](data-packs.md)       | Everyone                      | Locale packs, the field dictionary, `kaanal schema` and the contributor rule |
| [static-site.md](static-site.md)     | Frontend developers           | `site:`: serve your built frontend from the mock's own origin                |
| [collections.md](collections.md)     | Frontend developers           | `collection:`: in-memory routes that remember what you `POST`                |
| [paging.md](paging.md)               | Frontend developers           | Paged list routes: styles, headers, total, envelopes and limits              |
| [init.md](init.md)                   | Everyone                      | `kaanal init`: a starter project from an OpenAPI file                        |
| [demo.md](demo.md)                   | Everyone                      | `kaanal demo`: a demo shop with a chaos bar, no project needed               |

## Conventions for these docs

- Write for the user's goal ("make the database slow"), then show the command — not the other way round.
- Every command shown must work copy-pasted against the current binary.
- No contributor internals (crate names, file paths) except in clearly marked pointer boxes.
