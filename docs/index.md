# Kaanal Documentation

Kaanal is a single Rust binary that pretends to be your whole backend: it serves mock HTTP APIs from an OpenAPI spec, with realistic generated data, realistic per-hop timing, and chaos controls (latency, errors, node outages) you can flip live from a CLI or a small web page.

This folder has the contributor documentation; the user docs are on the site.

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

### The user docs site

The user-facing documentation is published at <https://kaanal.naveenr.in/docs/>; its Markdown lives in `site/src/content/docs/docs/` (see `site/README.md`).

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
