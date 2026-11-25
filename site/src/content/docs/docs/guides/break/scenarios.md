---
title: "Scenarios"
sidebar:
  order: 2
---

A scenario is a named bundle of chaos: "payments is down", "the database is slow and flaky", "checkout times out". You write it once, then switch it on and off with one command or one HTTP call, alone or stacked with others. A scenario holds **chaos** (latency, error rates and kinds, outages), **[pins](/docs/guides/break/pins/)** (force an endpoint to answer as loading, empty, error, slow or success), or both.

## Quick start

Put a scenario in `kaanal.yaml`:

```yaml
scenarios:
  payments-down:
    description: "Payments hard down"
    chaos:
      - { node: payments, down: true }
      - { endpoint: "POST /checkout", latency_ms: 800 }
      - { node: db, endpoint: "GET /orders", error_rate: 0.3, error_kind: reset }
```

```bash
kaanal up
kaanal scenario activate payments-down     # chaos is on
kaanal status                              # effective chaos and the active scenarios
kaanal scenario deactivate payments-down   # back to normal
```

## What an entry can say

Each entry names **where** it applies and sets **at least one** of the fields:

| Where                          | Meaning                                                                                          |
| ------------------------------ | ------------------------------------------------------------------------------------------------ |
| `node: db`                     | Every visit of that node.                                                                        |
| `endpoint: "GET /orders"`      | Once, at the front door of that endpoint, before its chain. A failure belongs to the entry node. |
| `node: db` + `endpoint: "..."` | Visits of that node only while it serves that endpoint.                                          |

| Field           | Rules                                                                                                                                                                                                                                                                                                                                                         |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `replica`       | integer, 0 to 63, with `node` only: one copy of a replicated node (no `endpoint`). The node must have that many `replicas`.                                                                                                                                                                                                                                   |
| `edge`          | `{ from, to }`: a link instead of a node (no `node`, `endpoint` or `replica`); the edge must exist. Takes `latency_ms`, `jitter_ms`, `error_rate` with `error_kind` `reset` or `hang`, `down` and `bandwidth_cap`.                                                                                                                                            |
| `jitter_ms`     | integer, 0 to 300 000, edge entries only: random extra latency of up to this much either way.                                                                                                                                                                                                                                                                 |
| `bandwidth_cap` | integer of at least 1 (bytes per second) or text with a unit such as `500KB/s` (see [nodes](/docs/reference/resources/#payload-cost)), edge entries only: lowers the edge's `bandwidth`; the edge must have one and the cap may not exceed it.                                                                                                                |
| `latency_ms`    | integer, 0 to 300 000. Added to every visit.                                                                                                                                                                                                                                                                                                                  |
| `error_rate`    | number 0.0 to 1.0. The chance each visit fails.                                                                                                                                                                                                                                                                                                               |
| `error_kind`    | `status` (default), `reset`, `hang` or `malformed`. See [concepts](/docs/how-it-works/#chaos).                                                                                                                                                                                                                                                                |
| `error_status`  | integer 500 to 504 (default 503). Checked whatever the kind.                                                                                                                                                                                                                                                                                                  |
| `from_ms`       | integer, 0 or more, with `down: true` and a `node` (no `endpoint` or `edge`): the instant of the run the outage begins, in milliseconds from its start. Without it the node or copy is down from before the run. A balancer's `detect` counts from it (see [nodes](/docs/reference/edges-and-failure/#when-a-call-fails-retries-abandon-breaker-and-detect)). |
| `down`          | `true`: every visit fails 503.                                                                                                                                                                                                                                                                                                                                |

`error_kind` and `error_status` belong to the **same entry's `error_rate`**. An entry that sets a kind or a status without a rate does nothing, and Kaanal tells you: `kaanal validate`, `kaanal scenario load` and the control API print a warning (it is not an error).

An entry repeating the same node+endpoint in one scenario, unknown keys, empty scenarios, out-of-range values, or unknown topology names fail on load (`file:line:col`). A `node` + `endpoint` pair never visited by that route's chain warns ("this entry never fires").

## Pins in a scenario

`pins:` sits next to `chaos:`, and `chaos:` is optional, so a scenario can be pins only:

```yaml
scenarios:
  orders-empty:
    pins:
      - { endpoint: "GET /orders", state: empty }
  checkout-fails:
    chaos:
      - { node: payments, latency_ms: 800 }
    pins:
      - { endpoint: "POST /checkout", state: error, status: 402, delay: 1s }
```

Each pin is `{ endpoint, state, status?, delay? }`. The states, defaults, layering and the `empty` rules are on [the pins page](/docs/guides/break/pins/). Pins follow the scenario: activating, deactivating, `--ttl`, `reset`, saving and the `X-Kaanal-Scenario` header all treat them like chaos entries. Pins count toward the 256 entries of a scenario. A `pins:` with no value (`null`) counts as absent.

`limits:` is reserved for a later feature and rejected with "not supported yet", so a file you share today will not break tomorrow. A `limits:` with no value (`null`) counts as absent; any other value, even `{}`, is rejected.

## Scenario files

Besides `scenarios:` in `kaanal.yaml`, a scenario can live in its own file `scenarios/NAME.yaml`, next to `kaanal.yaml`. The file name (without `.yaml`) is the name:

```yaml
# scenarios/slow-db.yaml
description: "Database is slow and flaky"
chaos:
  - { node: db, latency_ms: 300, error_rate: 0.1, error_status: 504 }
```

- One scenario per file, with `description` and `chaos`.
- Only `*.yaml`. Subdirectories, dotfiles and editor backup files are ignored; symlinks and `.yml` files are skipped with a warning.
- At most 128 scenarios, 256 entries each, 1 MiB per file.
- The same name inline and as a file is a load error that names both.
- The folder is found next to the config file you passed with `-c`, not in the current directory.

**Names:** lower-case letters, digits, `_` and `-`, starting with a letter or digit, up to 64 characters. `active`, `save`, `reset` and Windows device names (`con`, `prn`, `aux`, `nul`, `com1` to `com9`, `lpt1` to `lpt9`) are reserved.

## Stacking

Several scenarios can be active at once. They form a **stack**, in the order you activated them, and the chaos you set by hand (`kaanal chaos set`, `PUT /__kaanal/chaos`) is always the **top layer**.

- Activating a scenario that is already active moves it to the top of the scenarios.
- For the same field, **a higher layer wins over a lower one**. Within one layer, the more specific entry (node at endpoint before node) goes first.
- Latencies add up (clamped to 5 minutes per entry key: the node, endpoint and node-at-endpoint entries each clamp on their own). `down` from any layer takes effect. Error rates combine as independent chances, and each keeps its own kind and status: 30% `reset` in the top layer plus 30% `503` below gives about 30% resets and 21% 503s.
- With `mode: override` via the API, a hand-set entry replaces everything below it for that key (e.g. `down: false` calms a scenario). A node override also calms node-at-endpoint scenario entries for that node; a node-at-endpoint override only replaces other node-at-endpoint entries. Stays in override mode until a `PUT` sends `mode: merge` (see [control API](/docs/reference/control-api/)).
- Activating or deactivating a scenario does not change the random numbers a seeded run draws. To repeat a seeded run you also need to reset the request counters: see `reset` below.

## CLI

| Command                                                              | What it does                                                                                                                                         |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal scenario list`                                               | Every scenario with its source (`inline`, `file`, `runtime`), entry count, description, `(active)`.                                                  |
| `kaanal scenario show NAME`                                          | The chaos entries and pins of one scenario (`pin GET /x: error (status 404, delay 2s)`).                                                             |
| `kaanal scenario activate NAME [--ttl 30s]`                          | Put it on top of the stack. Prints the active scenarios. With `--ttl` it turns itself off later (see "Auto-expiring activations").                   |
| `kaanal scenario deactivate NAME`                                    | Take it off the stack (`ok: NAME is not active` if it was not).                                                                                      |
| `kaanal scenario set A B ...`                                        | Set the whole stack at once, bottom to top. With no names: deactivate all.                                                                           |
| `kaanal scenario clear`                                              | Deactivate every scenario. Chaos you set by hand stays.                                                                                              |
| `kaanal scenario active`                                             | Print the active scenarios, bottom to top.                                                                                                           |
| `kaanal scenario load FILE`                                          | Define a scenario from a YAML file (name = file name) in the running server and activate it. Errors show as `file:line:col` before anything is sent. |
| `kaanal scenario save NAME [--from SOURCE] [--force] [--clear-live]` | Write `scenarios/NAME.yaml`. `SOURCE` is `live`, `effective` or `scenario:OTHER`. See "Saving what you built" below.                                 |
| `kaanal reset`                                                       | Clean slate: see below.                                                                                                                              |
| `kaanal status`                                                      | Shows the **effective** chaos (scenarios plus what you set by hand) and the active scenario names.                                                   |

### Saving what you built

You found a nasty combination by hand. Keep it:

```bash
kaanal chaos set db --latency 300ms --error-rate 20% --kind reset
kaanal scenario save flaky-db --clear-live     # writes scenarios/flaky-db.yaml, clears the live chaos
kaanal scenario activate flaky-db              # same behaviour, now reusable
```

- `--from live` (default) saves what you set by hand. `--from effective` saves the compiled result of the whole stack (the "reproduce this for the bug report" case). `--from scenario:other` copies another scenario.
- Only the fields you actually set are written, so loading the file later adds no surprise defaults.
- A name that exists inline in `kaanal.yaml` is refused, even with `--force`. A scenario or file that already exists is refused without `--force`.
- `--clear-live` clears your hand-set chaos after writing, so activating the saved scenario does not count it twice.
- `--from effective` cannot save a key that has two or more failure kinds stacked on it, or a node-at-endpoint entry whose node also has chaos: it says which key. Save from `live` or from a scenario instead. A hand-set `mode: override` cannot be saved with `--from live` either; use `--from effective`.
- Scenarios you save are available immediately. Kaanal never rewrites `kaanal.yaml`.

### Reset

`kaanal reset` (or `POST /__kaanal/reset`) is for the start of every test:

- deactivates all scenarios and **forgets** the ones defined at runtime (`scenario load`, API `PUT`); scenarios from `kaanal.yaml` and from `scenarios/` stay defined;
- clears the chaos you set by hand;
- zeroes the request counters, so the same seeded run repeats;
- refills rate limits (a node that answered 429 starts with full tokens again).
- empties every [collection](/docs/guides/mock/collections/) (ids start at 1 again).

Queues and connection pools are not reset: requests already running finish normally. Call it between tests, not during one.

## HTTP API

All routes need the token like the rest of the [control API](/docs/reference/control-api/scenarios/). The ones you will use most:

```bash
TOKEN=$(jq -r .token .kaanal/run.json); PORT=$(jq -r .port .kaanal/run.json)
H="Authorization: Bearer $TOKEN"; B="http://127.0.0.1:$PORT/__kaanal"
curl -X POST   -H "$H" "$B/scenarios/payments-down/activate"
curl           -H "$H" "$B/scenarios/active"        # { "active": [...], "effective": {...} }
curl -X POST   -H "$H" "$B/reset"                    # 204
```

## Example: Playwright `beforeEach`

Start every test from a clean slate, then switch on only what the test needs:

```ts
// tests/kaanal.ts
import { readFileSync } from 'node:fs';

const run = JSON.parse(readFileSync('.kaanal/run.json', 'utf8')); // { host, port, token, pid }

export async function kaanal(path: string, init: RequestInit = {}) {
  const res = await fetch(`http://127.0.0.1:${run.port}/__kaanal${path}`, {
    ...init,
    headers: { Authorization: `Bearer ${run.token}`, 'Content-Type': 'application/json', ...init.headers },
  });
  if (!res.ok) throw new Error(`${init.method ?? 'GET'} ${path}: ${res.status} ${await res.text()}`);
  return res;
}
```

```ts
// tests/checkout.spec.ts
import { test, expect } from '@playwright/test';
import { kaanal } from './kaanal';

test.beforeEach(async () => {
  await kaanal('/reset', { method: 'POST' });          // no scenarios, no live chaos, counters at zero
});

test('checkout shows an error banner when payments is down', async ({ page }) => {
  await kaanal('/scenarios/payments-down/activate', { method: 'POST' });
  await page.goto('/checkout');
  await page.getByRole('button', { name: 'Pay' }).click();
  await expect(page.getByRole('alert')).toContainText('try again');
});

test('a slow database shows the loading state', async ({ page }) => {
  await kaanal('/chaos/db', { method: 'PUT', body: JSON.stringify({ latency_ms: 1500 }) });
  await page.goto('/orders');
  await expect(page.getByText('Loading')).toBeVisible();
});
```

**Parallel workers.** Global activation and `reset` change the backend for everyone. With Playwright's default parallel workers, scope each worker with the `X-Kaanal-Scenario` header instead of activating globally (see "Parallel test workers"), or run `workers: 1`, or start one Kaanal per worker in its own folder and port (the instance lock is `.kaanal/lock` in the folder where you ran `kaanal up`).

## Parallel test workers

Chaos state is global to an instance, but a single request can opt out of it. Send the header `X-Kaanal-Scenario` with the scenarios that request should see, and it is answered with exactly that stack and nothing else: no globally active scenarios, no hand-set (live) chaos.

```bash
curl -H 'X-Kaanal-Scenario: payments-down' http://127.0.0.1:8080/checkout
curl -H 'X-Kaanal-Scenario: slow-db,payments-down' http://127.0.0.1:8080/checkout   # later names win, like activation order
curl -H 'X-Kaanal-Scenario:' http://127.0.0.1:8080/checkout                          # empty = baseline, no chaos at all
```

| Case               | Behaviour                                                                                                                                                                                                                                                                                                                                                |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No header          | The request sees the global stack (what `kaanal scenario activate` built), as before.                                                                                                                                                                                                                                                                    |
| Empty header       | `X-Kaanal-Scenario:` is an explicit baseline. The request sees no chaos, whatever is active globally.                                                                                                                                                                                                                                                    |
| Names              | Comma separated, spaces around a name ignored, the last position wins when a name repeats (`a,b,a` is `b,a`). The stack is in header order, bottom to top.                                                                                                                                                                                               |
| Bad header         | `400 { "error": "bad scenario scope", "detail": "..." }` before running (no counter or chaos draw; still gets `X-Request-Id` and logs `rejected: bad-scope` in [observability](/docs/guides/observe/observability/)). Triggered by unknown/invalid names, empty names (`a,,b`), multiple header lines, non-ASCII values, `> 16` names, or `> 256` bytes. |
| Echo and events    | Responses echo `X-Kaanal-Scope` with the deduplicated stack (`b,a` for `a,b,a`, empty for empty header, absent without header; exposed via CORS). The [SSE stream](/docs/reference/control-api/live-and-events/#get-__kaanalevents) emits `scope` (`null`, `""`, or the stack). `404`/`405` and `/__kaanal` ignore the header.                           |
| Isolated vs shared | Scoping isolates chaos and scenario pins (ignoring `kaanal cycle` live pins), but route counters, rate limits, queues, and pools remain shared. For exact seeded replay, run one worker per instance.                                                                                                                                                    |
| Performance        | Compiled stacks cache after the first request (`RwLock` read + hash) and invalidate on any scenario/live change.                                                                                                                                                                                                                                         |

## Auto-expiring activations

A test that crashes after activating a scenario leaves it on. Give the activation a lease and Kaanal turns it off for you:

```bash
kaanal scenario activate payments-down --ttl 30s     # 100ms to 24h, e.g. 500ms, 30s, 5m, 2h
```

```bash
curl -X POST -H "$H" -H 'Content-Type: application/json' -d '{"ttl":"30s"}' "$B/scenarios/payments-down/activate"
# { "active": ["payments-down"], "expires_in_ms": { "payments-down": 30000 }, "effective": {...} }
```

- When the time is up the scenario is deactivated and an `expire` event is sent on the [event stream](/docs/reference/control-api/live-and-events/#get-__kaanalevents). Nothing else changes: other scenarios and hand-set chaos stay.
- Activating the same scenario again **replaces** the lease: with a new `--ttl` the clock restarts, with a plain `activate` (no body) the lease is removed and the scenario stays until you deactivate it.
- A lease lives only as long as the activation. Deactivating, `scenario set`, `scenario clear`, `reset` and deleting the scenario all drop it, so a stale lease can never switch off a scenario you activated later.
- `kaanal scenario list` shows the time left: `(active, expires in 28s)`, rounded up to whole seconds (`Nms` under one second). `scenario active` stays names-only.
- The CLI checks that `--ttl` is a duration from 100 ms to 24 h (exit 64 otherwise) before sending; the server enforces the same range (400) and rejects a body with other keys. Leases are in memory: they do not survive a restart, and neither does the activation.
- Several leases expire in deadline order, one `expire` event each.

## Timed chaos and timed scenarios

Two leases exist and they are independent:

|               | Leased scenario                                                           | Leased chaos                                                                                               |
| ------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Set with      | `scenario activate NAME --ttl 30s`                                        | `chaos set ... --for 30s`, `node down NODE --for 30s` (or `ttl` on `PUT /chaos`)                           |
| What runs out | The whole activation: the scenario is deactivated                         | The fields the command set (latency, errors, down, mode), cleared from the live entry                      |
| Granularity   | One lease per active scenario                                             | One lease per key and field group, so a permanent latency and a timed outage on one node expire separately |
| Event         | `expire` with the scenario `name`                                         | `expire` with `name: null` and a `chaos_key` such as `{"node":"db"}`                                       |
| Shown in      | `scenario list` (`expires in 28s`), `expires_in_ms` of `scenarios/active` | `node list` / `status` (`reverts in 27s`), `expires_in_ms` of `GET /chaos`                                 |

Neither touches the other: a chaos lease running out leaves every active scenario in place, and a scenario expiring leaves your hand-set chaos. Both are timed by the same server clock, so a 24 h scenario lease never delays a 1 s chaos lease.

Saving drops leases: `scenario save NAME --from live` (or `--from effective`) writes leased fields as permanent and warns (`N fields have a ttl; the file has none`), while leaving live timers running unless `clear_live` is set. See [control API](/docs/reference/control-api/health-and-chaos/#put-__kaanalchaosnode).

## Limits worth knowing

- No `limits:` yet (runtime limit overrides).
- The status page shows and edits only the chaos you set by hand, not scenarios (a scenario panel comes with the M2 UI). Use `kaanal status` or `GET /__kaanal/scenarios/active` to see the effective result.
- An activation without `--ttl` stays on until you deactivate it or `reset`. Use `--ttl` for tests that might crash, and start every test run with a reset.
- Scoping covers chaos and scenario pins only (see the shared-counter caveat above), and the header must be on the mock requests themselves; it has no effect on `/__kaanal`.
- Edits to `kaanal.yaml` or `scenarios/` need a restart (no hot reload yet). `kaanal scenario load` defines a scenario at runtime without one.
- Internals: [source/chaos.md](https://github.com/Naveen2070/kaanal/blob/main/docs/source/chaos.md).
