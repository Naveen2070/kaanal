---
title: "Chaos, live"
sidebar:
  order: 1
---

```bash
kaanal chaos set db --latency 300ms        # every db hop +300 ms
kaanal chaos set db --error-rate 10% --error 504
kaanal chaos set app --down                # hard outage: 503 for everything through app
kaanal status                              # see what's active
kaanal chaos clear db                      # back to normal
```

A failure you set can go unseen: a warm cache answers before the database is reached, and a node no route calls is never visited. If the run shows no failures although a node is down, read the trace (it names each hop), or break the cache too: `kaanal chaos set cache --down` makes every read reach the database. Studio shows this as a note under the chaos buttons and on Results. See [concepts](/docs/how-it-works/#chaos).

## Timed chaos

Add `--for` and the server undoes it for you, even if your script or terminal is gone:

```bash
kaanal node down db --for 30s              # db: down (reverts in 30s)
kaanal node list                           # db shows "down ... (reverts in 27s)"
kaanal chaos set db --latency 500ms --for 5m
kaanal node up db                          # bring it back early
```

- `--for` takes 100 ms to 24 h (`500ms`, `30s`, `5m`, `2h`); anything else is a usage error (exit 64) before anything is sent. It exists on `chaos set` and `node down` only.
- It leases what the command sets, field group by field group: latency, errors (rate, kind and status together), down and mode. `kaanal chaos set db --latency 200ms` (permanent) followed by `kaanal node down db --for 30s` expires only the outage; the latency stays.
- A command without `--for` makes the fields it sets permanent again: a plain `kaanal node down db` after a timed one keeps db down until you bring it up. Setting a field again with a new `--for` restarts its clock.
- When time is up the fields are **cleared**, not restored to an earlier value. `clear`, `reset` and `node up` end the lease too. Leases live in memory and are gone after a restart.
- `node up` only removes the live down flag. If an active scenario holds the node down, it prints `db is still down: held by an active scenario or override (see kaanal scenario active)` and exits 1, so `kaanal node up db && run-tests` does not run against a database that is still down.
- A timed chaos is not a [timed scenario](/docs/guides/break/scenarios/#timed-chaos-and-timed-scenarios): `--for` leases hand-set chaos, `scenario activate --ttl` leases an activation.

For a whole situation at once, define a [scenario](/docs/guides/break/scenarios/) and switch it on:

```bash
kaanal scenario activate payments-down     # defined under scenarios: in kaanal.yaml
kaanal scenario activate payments-down --ttl 30s   # turns itself off after 30 s
kaanal scenario deactivate payments-down
kaanal reset                               # start a test from a clean slate
```

Failures propagate upstream the way real ones do: a failing database makes the request fail with the database's status, after spending the time of the hops before it. The trace header names the failing node (`...,db.error=504`).
