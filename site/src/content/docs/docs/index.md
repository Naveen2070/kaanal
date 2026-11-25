---
title: Why Kaanal
description: What Kaanal is, the problems it solves, and where to start.
sidebar:
  order: 0
---

_Kaanal_ (கானல், Tamil for "mirage") is a single binary that pretends to be your backend: OpenAPI mocks with realistic
data, simulated service-to-database timing, and chaos you can switch on while it runs.

## The problems it solves

When you build frontends, mobile apps or microservices:

1. **Backends are late, incomplete or unstable.** Waiting for services blocks UI and integration work.
2. **A mock that answers instantly (0 ms) hides things.** Loading states, race conditions, spinners and latency bugs
   never show up.
3. **Failures are hard to test on demand.** Taking down a real database or producing a 504 gateway timeout is
   cumbersome.
4. **Mock data looks fake.** `"string"` and `0` in every field make a layout meaningless.

## What you get

Kaanal needs one YAML topology file and an OpenAPI 3.0 specification. From them it:

- serves realistic, coherent synthetic JSON (names, emails, phones, addresses, dates) in your locale, with an editable
  field dictionary: see [Data packs](/docs/guides/mock/data-packs/);
- simulates multi-hop latency: gateway, network, service and database time added up per request;
- injects live chaos (latency, error rates, a node down) from the command line, the HTTP API or Studio, the graphical
  editor;
- stays **deterministic** for a given seed, so tests are reproducible on CI and on your machine.

## Who it is for

- **Frontend and mobile developers and testers**: real-feeling data, slow and failing backends on demand, pinned
  loading, empty and error states.
- **CI and test authors**: a background instance with a control API and exit codes made for scripts.
- **People sizing a backend**: a simulator that finds where a design breaks under load and what would hold it.
- **People learning system design**: draw a topology (gateway, services, DB, cache, load balancer) and watch the numbers move — hop-by-hop latency, cache offloads, retries, breakers, and queue saturation. Every run is seeded for fair comparisons; start with [How Kaanal works](/docs/how-it-works/) and [Studio](/docs/guides/simulate/studio/).

## Where to start

- [Install](/docs/start/install/), then the [Quick start](/docs/start/quick-start/) (about five minutes).
- Want to see it first? [Try the demo shop](/docs/start/tour/).
- Wondering how it differs from WireMock, Mockoon, Prism, MockServer, Toxiproxy or MSW? See the [Comparison](/docs/comparison/).
- Know what you need? Pick a [how-to guide](/docs/guides/) or look it up in the [reference](/docs/reference/).
