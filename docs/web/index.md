# Kaanal user documentation (docs site)

This folder holds the **user-facing documentation** — the content of Kaanal's public docs site. It is written for people _using_ the tool, not changing it. (Contributors: you want [../coding-guide/](../coding-guide/index.md) and [../source/](../source/architecture.md).)

## Site structure

| Page                             | Audience                      | Contents                                                                    |
| -------------------------------- | ----------------------------- | --------------------------------------------------------------------------- |
| [user-guide.md](user-guide.md)   | Everyone                      | Install, `kaanal.yaml`, CLI, the embedded status page                       |
| [control-api.md](control-api.md) | Test authors, tooling writers | The `/__kaanal/*` HTTP API, request/response reference                      |
| [nodes.md](nodes.md)             | Everyone                      | Every node kind, its fields, the edge table and presets                     |
| [concepts.md](concepts.md)       | Everyone                      | Topology, call chains, chaos, determinism — what Kaanal actually simulates  |
| [web-ui.md](web-ui.md)           | Users of the status page      | The embedded `/__kaanal/ui` page, feature by feature                        |
| [endpoints.md](endpoints.md)     | Everyone                      | Endpoint chains, error maps, determinism and kaanal endpoints               |
| [auth.md](auth.md)               | Everyone                      | Login and JWKS from an auth node: config, tokens, keys, limits              |
| [capacity.md](capacity.md)       | Everyone                      | Slots, queues, rate limits, 429s and the `wait` trace piece                 |
| [scenarios.md](scenarios.md)     | Everyone                      | Named chaos scenarios: YAML, stacking, CLI, API, reset, test-runner recipes |

## Conventions for these docs

- Write for the user's goal ("make the database slow"), then show the command — not the other way round.
- Every command shown must work copy-pasted against the current binary.
- No contributor internals (crate names, file paths) except in clearly marked pointer boxes.
