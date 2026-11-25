---
title: "Example: drive chaos from a test setup"
sidebar:
  order: 10
---

## Quick curl chaos & scenario reset

```bash
TOKEN=$(jq -r .token .kaanal/run.json)
PORT=$(jq -r .port .kaanal/run.json)
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
     -d '{"down": true}' "http://127.0.0.1:$PORT/__kaanal/chaos/db"
# ... run the test ...
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/chaos"
```

Or start every test clean and switch on a [scenario](/docs/guides/break/scenarios/):

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/reset"
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/scenarios/payments-down/activate"
```

## Saving & reloading the draft

### `POST /__kaanal/draft/save`

Writes the draft text to the server's config file without touching the running system (that's Apply). Nothing is written to disk until Save.

- **Body (optional):** `{ "confirm_loss": true, "fix_scenarios": "none", "overwrite": false }`
- **Response:** `{ "ok": true, "written": ["kaanal.yaml"], "removed": [], "layout_written": true, "layout_note": null }`

#### `fix_scenarios` options

Controls what happens to `flagged` scenario files that no longer resolve against the draft:

| Value                  | Effect                                                                                                                                                                                                                                   |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`"none"`** (default) | Leaves them as-is (next `kaanal up` will fail until fixed).                                                                                                                                                                              |
| **`"remove"`**         | Deletes regular files directly inside the project's `scenarios/` folder (never symlinks) and lists them in `removed`. Returns `400` if the draft does not build.                                                                         |
| **`"rewrite"`**        | Updates `chaos[].node` and `chaos[].edge` ends in flagged files using recorded `rename_node` operations (chained in order, e.g. `a -> b -> c`). Returns `400` if no renames are recorded or a target sits inside an anchored YAML value. |

#### Atomicity, conflicts & path safety

| Topic                       | Behaviour                                                                                                                                                                                                                                                    |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **All-or-nothing writes**   | New files stage to temporary files first, then removals and the config write run in order with rollback on failure. If `PUT` updates the draft mid-save, the files to delete are computed from the exact text being saved.                                   |
| **Atomic file replacement** | Writes to an exclusive, unpredictable temp file in the same folder, flushes, and renames over the target (preserving permissions and symlinks). Unparseable YAML is saved as-is; `kaanal up` reports errors later.                                           |
| **Path rules**              | `spec:`, `data:`, and `endpoints:` must stay relative inside the project (no `..` or escaping links), otherwise Save returns `400`.                                                                                                                          |
| **Layout file**             | Writes `kaanal.layout.json` atomically after the config succeeds, dropping positions for removed nodes and keeping notes. Sets `layout_written` and `layout_note` (skipped without rolling back the config if the layout path is a symlink or folder moved). |

Conflicts and loss:

| Status  | When                                                                                                                                              |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **409** | With `"conflict": true` if the file on disk changed since Studio last read/wrote it (bypass with `"overwrite": true`, or discard with `/reload`). |
| **409** | With `"lost": [...]` if patches dropped data unless `"confirm_loss": true`.                                                                       |
| **409** | If started without a config file.                                                                                                                 |
| **500** | On write failure.                                                                                                                                 |

### `POST /__kaanal/draft/reload`

Discards the draft and reloads the config file and `kaanal.layout.json` from disk (no body).

- Resets `saved: true`, clears `lost` and recorded renames, and bumps `rev` if text changed.
- Keeps the currently applied system untouched (`dirty` reflects whether the reloaded file differs).
- Returns `409` if there is no config file or it cannot be read, or `400` if over 1 MiB.

## Blank start

`kaanal up --blank` (or `kaanal studio`) starts with no system applied (`version: 1\nnodes: {}\n`, `generation: 0`, `dirty: true`).

- Control API and `/__kaanal/ui` are available; all other routes return `404 {"error":"no system applied yet"}`, and topology/chaos/pin/scenario endpoints return `409`.
- Build the system via `add_node` patches and `POST /__kaanal/draft/apply` (empty drafts return `400`; the first valid Apply creates generation 1).
- `Save` writes to `-c` (default `kaanal.yaml`), checking for external edits since startup. Signing keys stay in memory until saved, after which `kaanal up` creates the project's key file.
