---
title: "Control API"
sidebar:
  order: 0
---

Everything under `/__kaanal/` — the single control plane used by the CLI, the status page, and your own scripts and test setups.

## Authentication & transport rules

| Rule             | Detail                                                                                                                                                                                                                                               |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Token**        | Every endpoint except `health` and `ui` requires `Authorization: Bearer <token>`. The token is printed by `kaanal up` and stored in `.kaanal/run.json`. On `/__kaanal/events` only, `?token=` is accepted (browser `EventSource` can't set headers). |
| **Host guard**   | The `Host` header must be `localhost`, `127.0.0.1` or `[::1]`, with the bound port (or none). Anything else returns `403`. This blocks DNS-rebinding attacks from browsers.                                                                          |
| **CORS**         | None on this namespace, ever: browsers on other origins cannot call it.                                                                                                                                                                              |
| **Headers**      | Responses carry `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.                                                                                                                                                                     |
| **Error format** | Errors are always JSON: `{ "error": "message" }`.                                                                                                                                                                                                    |

## Endpoints

## Pages

| Page                                                                                 | Covers                                                   |
| ------------------------------------------------------------------------------------ | -------------------------------------------------------- |
| [Health, topology and chaos](/docs/reference/control-api/health-and-chaos/)          | `GET /__kaanal/health`, `topology`, and the chaos routes |
| [Scenarios](/docs/reference/control-api/scenarios/)                                  | list, activate, deactivate, save                         |
| [Pins](/docs/reference/control-api/pins/)                                            | pinned UI states                                         |
| [Payload edits](/docs/reference/control-api/payload-edits/)                          | rewrite a route's response                               |
| [Draft](/docs/reference/control-api/draft/)                                          | the design draft: view, patch, validate, apply, save     |
| [Simulation](/docs/reference/control-api/simulation/)                                | runs, sweeps and plans                                   |
| [Logs and trace](/docs/reference/control-api/logs-and-trace/)                        | `GET /__kaanal/logs` and `trace/{id}`                    |
| [Live and events](/docs/reference/control-api/live-and-events/)                      | the live simulation and `GET /__kaanal/events`           |
| [Shutdown and the status page](/docs/reference/control-api/shutdown-and-ui/)         | `POST /__kaanal/shutdown`, `GET /__kaanal/ui`            |
| [Example: drive chaos from a test setup](/docs/reference/control-api/example-chaos/) | a worked test setup                                      |
