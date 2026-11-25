---
title: Comparison
description: How Kaanal differs from WireMock, Mockoon, Prism, MockServer, Toxiproxy and MSW, and when to pick each.
sidebar:
  order: 99
---

Several good tools sit near Kaanal. They overlap in places and answer different questions. This page says what each one
is built for, in its own words, and where Kaanal fits. Pick by the question you are asking, and combine tools when it
helps.

:::note
Facts about other tools come from each project's GitHub repository and README, read on 2026-11-25 (sources at the end).
Projects change; check their own docs before you decide. If something here is out of date, please
[open an issue](https://github.com/Naveen2070/kaanal/issues).
:::

## At a glance

| Tool                                              | What its README says it is                                                                                                                                        | Language, licence      | How you run it                                           |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- | -------------------------------------------------------- |
| [WireMock](https://wiremock.org/)                 | "A tool for mocking HTTP services": stubbing matched on URL, headers and body, record and playback, fault and delay injection, stateful behaviour                 | Java, Apache-2.0       | Library in tests, standalone process or container        |
| [Mockoon](https://mockoon.com/)                   | "The easiest and quickest way to run mock APIs locally": routes, OpenAPI compatibility, proxy forwarding, JSON templating, request logs                           | TypeScript, MIT        | Desktop app to design, CLI and Docker image to self-host |
| [Prism](https://stoplight.io/open-source/prism)   | A mock server and a validation proxy from OpenAPI v2, v3.x and Postman Collections                                                                                | TypeScript, Apache-2.0 | `npm` command-line tool                                  |
| [MockServer](https://www.mock-server.com/)        | "Mocks, records, and chaos-tests any HTTP(S) dependency": mock or proxy, inject latency, dropped connections and errors, many protocols on one port               | Java, Apache-2.0       | Docker, JAR, Helm, plus clients for several languages    |
| [Toxiproxy](https://github.com/Shopify/toxiproxy) | "A TCP proxy to simulate network and system conditions for chaos and resiliency testing": latency, bandwidth, timeout, reset, packet loss and more, set over HTTP | Go, MIT                | A proxy daemon between your app and a real service       |
| [MSW](https://mswjs.io/)                          | Intercepts requests inside your JavaScript app or tests so the code does not know what is mocked                                                                  | TypeScript, MIT        | A library in the browser or Node                         |
| **Kaanal**                                        | A single binary that pretends to be your backend: OpenAPI mocks, a simulated topology with timing and capacity, live chaos, and a load simulator                  | Rust, Apache-2.0       | One binary, `kaanal up`                                  |

## What is different about Kaanal

Most mock tools answer "what does this endpoint return?". Kaanal also answers "what does my backend _do_ when it is
slow, broken or busy?", because it models the system behind the endpoint:

- **A system, not only endpoints.** You describe gateways, services, databases, caches, load balancers, stores, auth and
  external APIs. A request walks the chain and spends time at every hop, and the `x-kaanal-trace` header shows where it
  went (see [Quick start](/docs/start/quick-start/)).
- **Capacity and failure behaviour.** Slots, queues, rate limits, CPUs, memory, retries, breakers and replicas change
  the numbers (see [Nodes reference](/docs/reference/nodes/)).
- **A simulator on the same model.** A load sweep finds where a design breaks and which node saturates first, and the
  sizing planner proposes what would hold a target load, with an estimated cost (see
  [Simulation](/docs/guides/simulate/simulation/)).
- **Deterministic.** The same seed gives the same data and the same timings, so a test or an experiment repeats exactly.
- **One binary, one port,** with Studio, a graphical editor, built in.

It is also a way to learn system design: change one thing in a small system and see what it does to latency and
failures.

## What Kaanal is not

From its own docs ([How Kaanal works](/docs/how-it-works/), [Simulation](/docs/guides/simulate/simulation/)):

- The database is a behavioural model of timing and failures, not Postgres. Nothing outside Kaanal connects to it.
- Timings come from the numbers you configure, not from a measured real system. Treat them as estimates, not
  predictions. A real system may saturate earlier than a run shows.

## Which one when

| If you need to                                                                                   | Look at                                  |
| ------------------------------------------------------------------------------------------------ | ---------------------------------------- |
| Isolate a Java test suite from a third-party HTTP service, with matching, record and playback    | WireMock                                 |
| Design mock routes in a desktop app and share the file                                           | Mockoon                                  |
| Check that a client or a server follows an OpenAPI document                                      | Prism (mock server and validation proxy) |
| Mock or proxy several protocols, or inject latency and dropped connections on an HTTP dependency | MockServer                               |
| Degrade the network between your app and a real service (latency, bandwidth, timeouts, resets)   | Toxiproxy                                |
| Mock inside your own JavaScript app or its tests, without a separate server                      | MSW                                      |
| Serve realistic mock data **and** see how a whole backend design behaves under failure and load  | Kaanal                                   |

They combine well. For example, you can run your frontend against Kaanal while you design and size the backend, and keep
WireMock, MockServer or MSW for protocol-level or in-code stubs in your tests.

## Sources

Read on 2026-11-25. Descriptions are quoted or paraphrased from these pages, and licences and languages come from the
GitHub repository metadata.

- WireMock: [github.com/wiremock/wiremock](https://github.com/wiremock/wiremock), [wiremock.org](https://wiremock.org/docs)
- Mockoon: [github.com/mockoon/mockoon](https://github.com/mockoon/mockoon), [mockoon.com](https://mockoon.com/)
- Prism: [github.com/stoplightio/prism](https://github.com/stoplightio/prism)
- MockServer: [github.com/mock-server/mockserver](https://github.com/mock-server/mockserver), [mock-server.com](https://www.mock-server.com/)
- Toxiproxy: [github.com/Shopify/toxiproxy](https://github.com/Shopify/toxiproxy)
- MSW: [github.com/mswjs/msw](https://github.com/mswjs/msw), [mswjs.io](https://mswjs.io/)
