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

A journey has an `endpoint` (the `"METHOD /path"` key, as in `endpoints:`), a `weight` above 0 (default 1), and optional `req_bytes` and `resp_bytes` (at most 1 GiB each). Bytes matter only when an edge has a `bandwidth` or a node has `per_kb`. An endpoint appears once per profile; give it a bigger weight instead of a second entry.

**What a journey is.** Each journey runs as its own open stream of independent requests with exponentially spaced arrivals (a Poisson stream). It is not a user session: there is no think time, no cart state, no "this user then does that". The mix tells you how much of each kind of request the system sees.

**What the result means.** A simulation is a relative what-if, not a prediction. Real traffic is burstier than a Poisson stream, so tails in a real system will be worse than a smooth stream shows.

Profile names follow the scenario-name rule: 1 to 64 characters of `a-z`, `0-9`, `_` or `-`, starting with a letter or a digit.

**Checked when the config loads:** the endpoint keys (`GET /a` and `get /a` are the same endpoint, and appear once), `duration` (above 0, at most 1h) and `warmup` (shorter than the duration). Whether an endpoint is a route of the system is a warning (`kaanal validate`, `up` and Apply show it), because the routes come from your specs; a run of a profile that names no route is refused. A request that shortens the duration drops a profile `warmup` that no longer fits and uses a tenth of the new duration.
