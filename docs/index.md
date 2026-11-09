# Kaanal Documentation

Kaanal is a single Rust binary that pretends to be your whole backend: it serves mock HTTP APIs from an OpenAPI spec, with realistic generated data, realistic per-hop timing, and chaos controls (latency, errors, node outages) you can flip live from a CLI or a small web page.

This documentation has three parts.

## Table of contents

### `source/` — for people working on the code

| Doc                                              | What it covers                                                              |
| ------------------------------------------------ | --------------------------------------------------------------------------- |
| [source/architecture.md](source/architecture.md) | How the pieces fit: crates, request flow, threading model, determinism      |
| [source/how-to.md](source/how-to.md)             | Build, run, test; how to add a node type, a CLI command, a control endpoint |
| [source/chaos.md](source/chaos.md)               | Failure kinds, `timeout:`, zombie hops, keyed chaos, layers, scenarios      |
| [source/tests.md](source/tests.md)               | Every test in every crate, explained                                        |
| [source/files/](source/files/)                   | One page per source file: what it does, what each function does             |

### `coding-guide/` — start here as a new contributor

| Doc                                            | What it covers                                                                       |
| ---------------------------------------------- | ------------------------------------------------------------------------------------ |
| [coding-guide/index.md](coding-guide/index.md) | Coding style, project rules, commit style, and a "I want to change X → read Y" table |

### `web/` — user-facing documentation (for the docs site)

| Doc                                      | What it covers                                                               |
| ---------------------------------------- | ---------------------------------------------------------------------------- |
| [web/index.md](web/index.md)             | What the docs site is and how it is organised                                |
| [web/user-guide.md](web/user-guide.md)   | Install, `kaanal.yaml`, CLI commands, the embedded UI                        |
| [web/control-api.md](web/control-api.md) | The `/__kaanal/*` HTTP control API reference                                 |
| [web/scenarios.md](web/scenarios.md)     | Named chaos scenarios: YAML, stacking, CLI, API, test recipes                |
| [web/concepts.md](web/concepts.md)       | Topology, call chains, chaos, determinism — explained for users              |
| [web/web-ui.md](web/web-ui.md)           | Embedded status page guide, live counters, security rules                    |
| [web/paging.md](web/paging.md)           | Paged list routes: styles, headers, `total:`, envelopes, limits              |
| [web/admin-mode.md](web/admin-mode.md)   | Studio's Admin mode: a real frontend in the live numbers, rewrite a response |

## The 30-second version

```bash
kaanal up -c kaanal.yaml        # serve mocks on http://localhost:8080
kaanal chaos set db --down      # take the database node down, live
kaanal chaos clear              # back to normal
kaanal scenario activate payments-down   # a named bundle of chaos from kaanal.yaml
kaanal reset                    # clean slate between tests
kaanal validate -c kaanal.yaml  # check config without starting
kaanal status                   # nodes and active chaos of the running instance
```

One binary, one port. Mock routes and the `/__kaanal` control API share it. Everything is deterministic for a given config `seed`.
