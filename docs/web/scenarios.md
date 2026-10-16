# Scenarios

A scenario is a named bundle of chaos: "payments is down", "the database is slow and flaky", "checkout times out". You write it once, then switch it on and off with one command or one HTTP call, alone or stacked with others. Scenarios hold **chaos only**: latency, error rates and kinds, outages. (Pinned response states and `kaanal cycle` come later.)

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

| Field          | Rules                                                                                  |
| -------------- | -------------------------------------------------------------------------------------- |
| `latency_ms`   | integer, 0 to 300 000. Added to every visit.                                           |
| `error_rate`   | number 0.0 to 1.0. The chance each visit fails.                                        |
| `error_kind`   | `status` (default), `reset`, `hang` or `malformed`. See [concepts](concepts.md#chaos). |
| `error_status` | integer 500 to 504 (default 503). Checked whatever the kind.                           |
| `down`         | `true`: every visit fails 503.                                                         |

`error_kind` and `error_status` belong to the **same entry's `error_rate`**. An entry that sets a kind or a status without a rate does nothing, and Kaanal tells you: `kaanal validate`, `kaanal scenario load` and the control API print a warning (it is not an error).

An entry with the same node and endpoint twice in one scenario is an error (merge them). Unknown keys, an empty `chaos:` list and values outside the limits are errors, shown as `file:line:col`. Names that do not exist in your topology are errors too, and `kaanal validate` checks them. A `node` + `endpoint` pair the endpoint's chain never visits is only a warning ("this entry never fires").

`limits:` and `pins:` keys are reserved for later features and rejected with "not supported yet", so a file you share today will not break tomorrow. A `limits:` or `pins:` with no value (`null`) counts as absent; any other value, even `{}`, is rejected.

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
- Through the API, a hand-set entry with `mode: override` replaces everything below it for that key, so a live `down: false` can calm a scenario. A node override also calms that node's node-at-endpoint entries from scenarios. A node-at-endpoint override replaces only the other node-at-endpoint entries of that pair; it cannot calm chaos set on the node itself (use a node override for that). The mode sticks until a later `PUT` says `mode: merge`. See [the control API](control-api.md).
- Activating or deactivating a scenario does not change the random numbers a seeded run draws. To repeat a seeded run you also need to reset the request counters: see `reset` below.

## CLI

| Command                                                                                       | What it does                                                                                                                                         |
| --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal scenario list`                                                                        | Every scenario with its source (`inline`, `file`, `runtime`), entry count, description, `(active)`.                                                  |
| `kaanal scenario show NAME`                                                                   | The entries of one scenario.                                                                                                                         |
| `kaanal scenario activate NAME`                                                               | Put it on top of the stack. Prints the active scenarios.                                                                                             |
| `kaanal scenario deactivate NAME`                                                             | Take it off the stack (`ok: NAME is not active` if it was not).                                                                                      |
| `kaanal scenario set A B ...`                                                                 | Set the whole stack at once, bottom to top. With no names: deactivate all.                                                                           |
| `kaanal scenario clear`                                                                       | Deactivate every scenario. Chaos you set by hand stays.                                                                                              |
| `kaanal scenario active`                                                                      | Print the active scenarios, bottom to top.                                                                                                           |
| `kaanal scenario load FILE`                                                                   | Define a scenario from a YAML file (name = file name) in the running server and activate it. Errors show as `file:line:col` before anything is sent. |
| `kaanal scenario save NAME [--from live\|effective\|scenario:OTHER] [--force] [--clear-live]` | Write `scenarios/NAME.yaml`. See "Saving what you built" below.                                                                                      |
| `kaanal reset`                                                                                | Clean slate: see below.                                                                                                                              |
| `kaanal status`                                                                               | Shows the **effective** chaos (scenarios plus what you set by hand) and the active scenario names.                                                   |

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

Queues and connection pools are not reset: requests already running finish normally. Call it between tests, not during one.

## HTTP API

All routes need the token like the rest of the [control API](control-api.md#scenarios). The ones you will use most:

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

**One instance per worker.** Kaanal's chaos state is global to a running instance. With Playwright's default parallel workers, one test's `reset` or activation would change the others' backend. Run with `workers: 1`, or start one Kaanal per worker in its own folder and port (the instance lock is `.kaanal/lock` in the folder where you ran `kaanal up`). Per-request scenario selection (a header or cookie) for shared instances is planned.

## Limits worth knowing

- Chaos only: no `limits:` or `pins:` yet.
- The status page shows and edits only the chaos you set by hand, not scenarios (a scenario panel comes with the M2 UI). Use `kaanal status` or `GET /__kaanal/scenarios/active` to see the effective result.
- Scenarios cannot expire on their own yet (no `ttl`): a test that crashes after activating leaves the scenario on until you `reset`. Start every test run with a reset.
- Edits to `kaanal.yaml` or `scenarios/` need a restart (no hot reload yet). `kaanal scenario load` defines a scenario at runtime without one.
- Internals: [source/chaos.md](../source/chaos.md).
