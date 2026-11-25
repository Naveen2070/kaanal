---
title: "Example: drive chaos from a test setup"
sidebar:
  order: 10
---

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

`POST /__kaanal/draft/save` writes the draft text to the config file the server was started from. The body is optional: `{ "confirm_loss": true, "fix_scenarios": "none", "overwrite": false }`. It answers `{ "ok": true, "written": ["kaanal.yaml"], "removed": [], "layout_written": true, "layout_note": null }`. Save does not touch the running system (that is Apply), and nothing is written to disk until Save: patches, validate and Apply leave the file alone.

- **`fix_scenarios`** says what to do with the scenario files the draft no longer resolves against (the `flagged` ones): `"none"` (the default) leaves them, so the next `kaanal up` fails on them until you fix them; `"remove"` deletes them (only regular files directly in the project's own `scenarios/` folder, which must not itself be a link; they are listed under `removed` as `scenarios/NAME`), so the next `kaanal up` loads; `"rewrite"` points the flagged scenario files at the renamed node: the draft remembers the `rename_node` operations made since its text was set (a whole-text `PUT` forgets them), and Save changes each of those names in the files' `chaos[].node` and `chaos[].edge` ends, chained in order (`a -> b -> c` ends at `c`), as part of the same all-or-none write; a file that names none of them stays flagged, a file with a name inside an anchored value stops the Save (`400`, nothing written), and `rewrite` with no rename recorded is `400`. The files to delete are worked out from the very text that is written, so a `PUT` that lands during a Save cannot change which files go (the saved text is what was checked; the newer draft stays and the next Save writes it). `remove` on a draft that does not build is `400`. A Save that is refused (conflict, `lost`) deletes nothing.
- **Every file changes or none.** The new files are written to temporary files first; then the removals and the config are made in order (the config last), each remembering what it replaced, and if one fails the ones already made are put back (if even that fails, the error names the files that could not be put back). A file that cannot be read stops the Save before anything changes.
- The file is replaced **atomically**: the text goes to a temporary file in the same folder, is flushed to disk, and is renamed over the config, so a reader (or a crash) sees the old file or the new one, never half. The file keeps its permissions, and a config that is a link is written through to its target (the link stays).
- `409` with `"conflict": true` when the file on disk is not what Studio last read or wrote (an editor or `git checkout` changed it, it was deleted, or it appeared where there was none). Nothing is written. Two ways out: send the same Save with `"overwrite": true` (the draft replaces the file, a deleted file is written again), or `POST /__kaanal/draft/reload` (below) to take the file and drop the draft. What is compared is the text of the file, so touching it without changing it is not a conflict.
- `409` with `"lost": [...]` when patches could not keep something (see Patches) and `confirm_loss` is not true; nothing is written. With `confirm_loss: true` the draft is saved as it stands, and `lost` is cleared.
- `409` when the server was not started from a config file; `500` when the write fails (every file is as it was, and no temporary file is left, unless the error says some file could not be put back).
- **Save holds the same path rule as validate and Apply.** A draft whose `spec:`, `data:` or `endpoints:` is absolute, has `..`, or leaves the project through a link is `400` and nothing is written (otherwise the next `kaanal up`, which takes paths as written, would trust what the API stored). Text that is not YAML at all is not checked: nobody can load it.
- The temporary file has a name that cannot be guessed and is created exclusively, so a file or link planted at that path is never written through.
- The draft text is saved as it is, valid or not (a half-typed file can be saved); `kaanal up` on a file that does not load fails with its usual located error.
- **The layout** is written after the config succeeds, to `kaanal.layout.json` in the config file's folder, by the same temporary-file-and-rename, and the answer gains `"layout_written": true|false` and `"layout_note"` (why not, or `null`). Positions of nodes the saved text does not define are dropped (when it is a valid config). Notes go in the same file. No positions and no notes writes nothing and removes nothing (unless the file held notes, which are then removed). If `kaanal.layout.json` is a link or not a regular file, it is not replaced, and if the config's folder resolves somewhere else than when the Save began, nothing is written there; either way the config write stands (it is not rolled back) and `layout_note` says so.

`POST /__kaanal/draft/reload` throws the draft away and reads the config file (and `kaanal.layout.json`, positions and notes) again, with no body. The draft is then the file (`saved` is true, `lost` and the recorded renames are cleared), `rev` moves when the text changed, and what is applied stays applied, so `dirty` says whether the file differs from it. It answers the draft view. `409` when the server has no config file or the file cannot be read (it is missing, say); `400` over 1 MiB. The draft is unchanged on an error.

**Blank start.** `kaanal up --blank` (or `kaanal studio`) starts with no system. The control API and `/__kaanal/ui` work; every other path answers `404 {"error":"no system applied yet"}`; what needs a system (topology, chaos, pins, scenarios) answers `409 no system applied yet; apply a draft`. The draft is `version: 1\nnodes: {}\n`, `generation` is `0` and `dirty` is true. Build it with `add_node` and the other patches, and `POST /__kaanal/draft/apply`: an empty draft is `400` with the topology's own message (an entry node is needed), and the first Apply that builds makes generation 1. `Save` writes the file named with `-c` (default `kaanal.yaml`): if that file exists when the server starts, Save writes over it only if it has not changed since then, like any Save. A blank start keeps a signing key in memory, so an `auth` node added later works; the project's own key file is made by `kaanal up` once the file is saved.
