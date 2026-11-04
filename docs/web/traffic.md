# Traffic profiles

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

A journey has an `endpoint` (the `"METHOD /path"` key, as in `endpoints:`), a `weight` above 0 (default 1), and optional `req_bytes` and `resp_bytes` (at most 1 GiB each). A size is a whole number of bytes (`2048`) or text with a unit (`2KB`, `1.5 MB`, `512B`): units `B`, `KB`, `MB`, `GB` in any case (`KiB`, `MiB`, `GiB` mean the same), with **KB = 1024 bytes** as in Studio, and a fraction rounds to the nearest byte. A size takes no `/s`, and a negative number, a missing number or an unknown unit is a load error naming the field and the units; the 1 GiB limit applies to the converted number (`req_bytes: 2GB` fails). Bytes matter only when an edge or a node has a `bandwidth`, a node has `per_kb`, or a storage node has a `throughput`. A route that takes or returns a **file** (a `multipart/*` form, an octet stream, an image, a PDF, a zip, or a `format: binary` schema) has no size in the OpenAPI document, so `kaanal validate` and `kaanal init` warn that you should set `req_bytes` or `resp_bytes` for it; a response `example` does set the default `resp_bytes`, because the generated body is the example. The bodies on the wire also carry the headers of [`header_bytes`](nodes.md#payload-cost), and an edge or step can change what a call carries (`sends`, `returns`). An endpoint appears once per profile; give it a bigger weight instead of a second entry.

**What a journey is.** Each journey runs as its own open stream of independent requests with exponentially spaced arrivals (a Poisson stream). It is not a user session: there is no think time, no cart state, no "this user then does that". The mix tells you how much of each kind of request the system sees.

**What the result means.** A simulation is a relative what-if, not a prediction. Real traffic is burstier than a Poisson stream, so tails in a real system will be worse than a smooth stream shows.

**Files written for you.** `kaanal init` and `kaanal demo --eject` write one profile named `peak`: `rps: 100`, `duration: 30s`, and one journey per route of the file's `endpoints:` (weight 3 for a `GET`, 1 for `POST`, `PUT`, `PATCH` and `DELETE`). The numbers are estimates; edit them. A spec with more than 64 routes gets journeys for the first 64, and a spec with no route that gets an entry gets no `traffic:` section at all (a profile needs a journey). Files that already exist are never changed.

**No profile.** The CLI has no built-in profile. A project without `traffic:` cannot run `kaanal sim run` until you add a section or pass a file with `-f`. The messages say what to do (the same text comes back as the `404` of `POST /__kaanal/sim/run`; the names are sorted, at most ten, then `...`):

| You ran                                                  | Message                                                                                                                              |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `kaanal sim run`, the file defines no profile            | `no traffic profile is defined: add a traffic: section to kaanal.yaml or pass -f profile.yaml (see docs/web/traffic.md)`             |
| `kaanal sim run`, the file defines profiles              | `no traffic profile named; name one with -p: peak, other`                                                                            |
| `kaanal sim run -p x`, the file has no profile named `x` | `no traffic profile named 'x'; defined: peak` (with none defined: `...; none is defined: add a traffic: section to kaanal.yaml ...`) |

Naming no profile is still a usage error (exit 64); a wrong name is a runtime error (exit 1). Studio sends its own profile and does not read these messages.

Profile names follow the scenario-name rule: 1 to 64 characters of `a-z`, `0-9`, `_` or `-`, starting with a letter or a digit.

**Checked when the config loads:** the endpoint keys (`GET /a` and `get /a` are the same endpoint, and appear once), `duration` (above 0, at most 1h) and `warmup` (shorter than the duration). Whether an endpoint is a route of the system is a warning (`kaanal validate`, `up` and Apply show it), because the routes come from your specs; a run of a profile that names no route is refused. A request that shortens the duration drops a profile `warmup` that no longer fits and uses a tenth of the new duration.
