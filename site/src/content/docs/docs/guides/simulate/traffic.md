---
title: "Traffic profiles"
sidebar:
  order: 2
---

A traffic profile describes the load you want the simulator to run: a total rate and a weighted mix of endpoints. It goes in `kaanal.yaml` under `traffic:`, one named profile per entry.

```yaml
traffic:
  peak:
    rps: 200
    duration: 2m
    journeys:
      - { endpoint: "GET /products", weight: 70 }
      - { endpoint: "GET /cart", weight: 20 }
      - { endpoint: "POST /orders", weight: 10, req_bytes: 2048, resp_bytes: 512 }
```

| Field      | Meaning                                                                |
| ---------- | ---------------------------------------------------------------------- |
| `rps`      | Total requests per second, 0.000001 to 1000000.                        |
| `journeys` | The mix: 1 to 64 entries. Each gets `rps × weight ÷ sum of weights`.   |
| `duration` | Simulated time of a run (default 60s). A run request may override it.  |
| `warmup`   | Simulated time before results count (default a tenth of the duration). |

## Journeys & payload sizes

Each journey specifies:

| Field                     | Meaning                                       |
| ------------------------- | --------------------------------------------- |
| `endpoint`                | `"METHOD /path"`, appearing once per profile. |
| `weight`                  | `weight > 0` (default `1`).                   |
| `req_bytes`, `resp_bytes` | Optional payload sizes, up to `1 GiB` each.   |

- **Size units:** whole bytes (`2048`) or text with units (`2KB`, `1.5 MB`, `512B`, `KiB`/`MiB`/`GiB`; **KB = 1024 bytes**, fractional values round to nearest byte). No `/s` suffix; negative, missing, or unknown units fail load.
- **When bytes matter:** only when an edge/node has `bandwidth`, a node has `per_kb`, or a storage node has `throughput`.
- **Binary routes & headers:** routes taking/returning files (`multipart/*`, octet-stream, images, PDFs, zips, `format: binary`) have no OpenAPI size, so `validate` and `init` warn you to set `req_bytes` / `resp_bytes` (response `example` sets default `resp_bytes`). Wire bodies also include [`header_bytes`](/docs/reference/resources/#payload-cost), and edges/steps can override sizes via `sends` / `returns`.

## How journeys execute

- **Poisson arrivals:** each journey runs as an independent open stream with exponentially spaced arrivals. There are no user sessions, think time, or cart state — just the request mix.
- **Interpreting results:** simulations are relative what-ifs, not predictions. Real traffic is burstier than Poisson, so production tails will be worse than a smooth stream shows.

## Generated profiles & missing profiles

`kaanal init` and `kaanal demo --eject` write a starter `peak` profile (`rps: 100`, `duration: 30s`, up to 64 routes with weight 3 for `GET` and 1 for writes) as estimates you should edit. Existing files are never overwritten; specs with no matching routes omit `traffic:`.

The CLI has no built-in profile. Without `traffic:`, `kaanal sim run` fails unless you pass `-f profile.yaml` (Studio sends its own profile):

| You ran                                                  | Message                                                                                                                                |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal sim run`, the file defines no profile            | `no traffic profile is defined: add a traffic: section to kaanal.yaml (a named profile with rps and journeys) or pass -f profile.yaml` |
| `kaanal sim run`, the file defines profiles              | `no traffic profile named; name one with -p: peak, other`                                                                              |
| `kaanal sim run -p x`, the file has no profile named `x` | `no traffic profile named 'x'; defined: peak` (with none defined: `...; none is defined: add a traffic: section to kaanal.yaml ...`)   |

Missing `-p` is exit `64` (usage); an unknown profile name is exit `1` (runtime). Same messages return as `404` on `POST /__kaanal/sim/run` (names sorted, capped at 10 + `...`).

## Validation rules

- **Profile names:** 1–64 chars of `a-z`, `0-9`, `_`, `-`, starting with a letter or digit.
- **Load-time checks:** endpoint keys are case-insensitive and unique per profile, `duration` is `> 0` and `<= 1h`, and `warmup < duration` (shortening `duration` via a run request resets an oversized `warmup` to `duration / 10`).
- **Route warnings:** unknown endpoints warn on `validate`, `up`, and Apply; running a profile with zero valid routes is refused.
