---
title: "Draft"
sidebar:
  order: 5
---

The draft is the text of `kaanal.yaml` being edited, kept apart from the running system and from disk. Editing it changes neither; **Apply** and **Save** do. It starts as the config file's text as `kaanal up` read it (comments and line endings included); a server started with no file starts with an empty draft.

## Reading the draft (`GET /__kaanal/draft`, `GET /__kaanal/demo`)

`GET /__kaanal/draft` returns:

```json
{
  "text": "...", "dirty": false, "generation": 1, "lost": [], "rev": 0,
  "doc": {}, "doc_error": null, "project_id": "...", "project_name": "...",
  "kind_defaults": {}, "kinds": [], "layout": {}, "layout_note": null,
  "layout_dirty": false, "canvas_notes": [], "saved": true, "renames": [], "boot": "..."
}
```

| Field                               | Meaning                                                                                                                                                                                                                                                                                      |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`text`** / `dirty` / `generation` | `dirty` is `true` when `text` differs from the last applied system; `generation` counts applied systems (`0` before the first Apply of a blank start).                                                                                                                                       |
| **`doc`** / `doc_error`             | `doc` is the draft parsed as JSON, or `null` with `doc_error` (`line`, `col`, `message`, same as `POST /draft/validate`) when invalid; HTTP status stays `200`.                                                                                                                              |
| **`project_name`** / `project_id`   | `project_name` is the config folder's base name (max 80 chars, or `null` with no file); `project_id` is 16 hex chars hashed from the config path (`none` with no file).                                                                                                                      |
| **`boot`** / `rev`                  | `boot` is a per-process random id; `rev` resets to `0` on restart, so a changed `boot` tells Studio the server restarted.                                                                                                                                                                    |
| **`renames`**                       | `[from, to]` pairs from `rename_node` edits since the text was set (cleared on whole-text `PUT`, Reload, or restart), used by `fix_scenarios: "rewrite"` on Save.                                                                                                                            |
| **`layout_dirty`**                  | `true` when positions or canvas notes differ from `kaanal.layout.json` (neither makes `dirty` true).                                                                                                                                                                                         |
| **`kind_defaults`** / `kinds`       | Per-kind defaults Studio creates, plus engine rules (`[{ "kind": "gateway", "callees": [...], "service_time": true }, ...]`). Studio only offers wires `callees` allows and shows a Time box only when `service_time` is `true` (`storage` is `false`: it uses `first_byte` + `throughput`). |

`GET /__kaanal/demo` returns `{ "text": ... }`, Studio's self-contained starter system (no spec or site file; includes a `peak` profile).

## Updating the draft (`PUT /__kaanal/draft`)

Requires the control token. `PUT` answers with the same view as `GET /__kaanal/draft` (last writer wins).

- **`{ "text": "..." }`**: replaces the draft without validating (half-typed files are allowed). Over `1 MiB` (`1048576` bytes of `text`, body up to ~6 MiB for escaped control chars) returns `400` and leaves the draft unchanged.
- **Validation**: sending both `text` and `patch`, or none of `text`, `patch`, `layout`, and `canvas_notes`, returns `400`; wrong field types return `422`.

### Patches (`{ "patch": [...] }`)

Applies edits sequentially on the YAML text, touching only changed bytes so comments, key order, and flow style (`{ from: a, to: b }`) stay intact:

| Operation    | Effect                                                                                                                                                                                                                 |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`set`**    | Replace the value at `path`.                                                                                                                                                                                           |
| **`insert`** | Last `path` step is the new key under the parent mapping (e.g. `["nodes", "cache"]`). Keys are written bare only when safe, otherwise double-quoted.                                                                   |
| **`append`** | Append `value` to a list at `path` using the list's style (`-` block item or `, item` flow item). Missing paths return `400`; non-lists, empty block lists, or anchored lists are left unchanged and listed in `lost`. |
| **`swap`**   | Exchange two list items (`path` and `with` in the same list), keeping each item's comments. Missing `with` or different lists return `400`; anchored items stay unchanged and go to `lost`.                            |
| **`remove`** | Delete a mapping entry or list item.                                                                                                                                                                                   |

#### Paths and values

`path` steps are keys or numeric list indices. JSON `value`s serialize as clean YAML (`db`, `2ms`, `"yes"`, `"8"`, `{ type: cache, flavor: redis }`, `[a, b]`). `"value": null` sets null; only a missing `value` field is an error.

#### Lost edits and refusals

- Anchored/aliased values (and edits inside them) are refused or left untouched and listed in `lost`.
- A comment directly above a removed entry stays behind; a comment between `key:` and a block value moves with the value.
- Removing the last entry of a block mapping leaves it empty (`null`).
- Duplicate keys anywhere in the draft, missing paths, existing keys on `insert`, or non-YAML drafts return `400` with no changes.
- Replacing the whole `text` clears `lost`.

### Building operations (`add_node`, `rename_node`)

#### `add_node`

`{ "op": "add_node", "name": "cache1", "kind": "cache" }` adds a node with Studio defaults (see [nodes](/docs/reference/nodes/#nodes-added-in-studio)). Unknown kind, missing fields, or duplicate name returns `400`.

| Kind           | Studio default                            |
| -------------- | ----------------------------------------- |
| `gateway`      | `concurrency: 256`                        |
| `service`      | `32`                                      |
| `monolith`     | `64`                                      |
| `database`     | `pool: { size: 20 }` + `flavor: postgres` |
| `cache`        | `flavor: redis`                           |
| `external-api` | `pool: { size: 10 }`                      |
| `auth`         | `32`                                      |

#### `rename_node`

`{ "op": "rename_node", "from": "db", "to": "store" }` updates `nodes:`, `edges`, node `chains:`, inline `endpoints:` `chain:` steps, `error_map` keys, and inline `scenarios:`, returning `touched`, one entry per place changed (`"nodes.db"`, `"edges[1].to"`, ...).

- Refuses (`400`) if missing, duplicate/empty/same, or inside an anchor.
- External `endpoints:` files appear in `notes`; `scenarios/*.yaml` flag on Apply; Apply drops live chaos on the old name.

### Layout & canvas notes (`kaanal.layout.json`)

- **Layout:** `{ "version": 1, "nodes": { "gw": [120, 80] } }`. `PUT` with `layout` replaces the map (`±1000000` coords, max `2000` positions, `<= 256 KiB`; else `400`). Never bumps `rev` or `dirty`; `rename_node` moves positions; clearing all never deletes an existing `kaanal.layout.json` (`layout_note` explains).
- **Canvas notes:** `{ "id": "note:1", "text": "...", "at": [x, y] }` under `notes` in `kaanal.layout.json` (max `50` notes, `<= 64` byte ids, `<= 500` chars, `<= 256 KiB`; else `400`). Never bumps `rev`, `dirty`, or undo; `rename_node` leaves notes untouched; invalid files on load block overwriting (`layout_written: false`).

### Project path rules

Drafts arriving via the API must keep `spec:`, `data:`, and `endpoints:` as relative paths inside the project folder (no absolute paths, `..`, or symlinks escaping the project; spec/endpoints files must be regular files `<= 16 MiB`). `kaanal up` itself allows paths outside the project, but a project with `spec: ../shared/api.yaml` starts and cannot be Applied until the file is inside the project or linked into it.

## Validating (`POST /__kaanal/draft/validate`)

Builds the draft with the same loader as `kaanal up` (parse, topology, specs, endpoints, scenarios) without touching the running system or storing anything. Optional `{ "text": "..." }` validates that text instead (over `1 MiB` returns `400`).

| Result      | Response                                                                                                                                     | Notes                                                                                              |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| **Valid**   | `200 { "ok": true, "diagnostics": [], "warnings": [...], "flagged": [...] }`                                                                 |                                                                                                    |
| **Invalid** | `200 { "ok": false, "diagnostics": [{ "file": null, "line": 9, "col": 14, "path": ["edges", 1, "to"], "message": "..." }], "warnings": [] }` | Stops at the first error (`line` is 1-based, `col` 0-based, `file` set for spec/endpoints errors). |

## Applying (`POST /__kaanal/draft/apply`)

Builds the draft, **holds new mock traffic**, drains in-flight mock requests (optional `{ "timeout_ms": 10000 }`, max `60000`), swaps the running system, and resumes traffic. Control-API requests are never held.

```json
{
  "ok": true,
  "generation": 2,
  "kept": { "live_chaos": 1, "active_scenarios": ["s1"], "runtime_scenarios": ["x"], "pins": 1 },
  "dropped": { "live_chaos": ["node 'db'"], "scenario_chaos": [], "active_scenarios": ["payments-down"], "pins": [] },
  "flagged": []
}
```

- **What resets**: topology, routes, endpoints, specs, data, auth issuer, and engine (`generation` increments). Route counters, `collection:` items, rate-limit buckets, and queues reset. Rebuilds from disk (picks up edited spec/data/scenario files even if `text` is unchanged). Control token, key files, and port stay.
- **What carries over**: live chaos, `ttl` leases, runtime scenarios (`PUT /scenarios/{name}`), active scenarios, and live pins where their target node, edge, replica, or endpoint still exists (and `empty` pins where an empty form still exists). Dropped items are listed under `dropped` (`dropped.live_chaos` for chaos, `dropped.scenario_chaos` for runtime scenarios as `scenario NAME: ...`; including `bandwidth_cap` removed from an edge with insufficient bandwidth, or pins/chaos on static/synthesized-HEAD routes). Draft-defined scenarios override same-named runtime scenarios.
- **`flagged`**: `scenarios/*.yaml` files referencing missing nodes/edges/endpoints are excluded from the new system and listed in `flagged` as `[ { "file": "...scenarios/f.yaml", "scenario": "f", "message": "..." } ]` (the file stays on disk; startup remains strict on next `kaanal up`; inline `scenarios:` remain strict).
- **Concurrency & errors**: one Apply at a time (second waits); `dirty` becomes `false` and `events` emits `applied`. `400` on build failure or bad `timeout_ms`/JSON (empty body is accepted). `409` on drain timeout (`drain timed out; the old system keeps running`) or concurrent system swap during a control call (`the system changed while this request ran; retry`). Chaos reads (`GET /chaos`, `GET /scenarios/active`, `GET /scenarios/{name}`) never return that `409`.
