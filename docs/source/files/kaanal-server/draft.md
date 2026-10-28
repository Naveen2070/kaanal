# `crates/kaanal-server/src/draft/mod.rs`

The Studio draft: the text of the `kaanal.yaml` being edited, apart from the running system.

## What it does

The draft is canonical YAML text, so comments, order and line endings survive an edit (Studio step 2, spec section 5). Replacing it touches neither the running system nor the disk; Apply and Save (later tasks) are what do. One draft per server, last writer wins (single user).

## Types

| Item | Kind | What it does |
| --- | --- | --- |
| `MAX_DRAFT_BYTES` | const | 1 MiB (1048576): the longest text `PUT /draft` accepts. |
| `DraftSource` | struct | `file` and `text`: what a draft starts from. `kaanal up` passes the config file and the text it read; exported through `ServeOpts.draft`. |
| `Draft` | struct | `text` (being edited) and `applied` (the text of the generation in force). `Draft::from_source(Option<DraftSource>)` starts clean, or empty with no source; `dirty()` is `text != applied`. Held in `AppState.draft` (`Mutex<Draft>`). |

`draft/apply.rs` turns draft text into a system:

| Item | Kind | What it does |
| --- | --- | --- |
| `Diagnostic` | struct | One problem placed in the draft: `file` (when not `kaanal.yaml`), `line` (1-based), `col` (0-based), `path` (`["edges", 1, "to"]`), `message`. |
| `build_draft(text, dir, drawn_seed, auth_key, port)` | fn | `Config::from_yaml`, then `Generation::build` (lenient) and the auth issuer (so a draft with an auth node and no key fails in validate as in Apply), with no engine started; returns the generation and its scenarios, or the first problem as a `Diagnostic`. Apply (T6) builds the same way before it swaps. |

`draft/gate.rs` is the admission gate Apply uses to stop mock traffic for the length of a drain:

| Item | Kind | What it does |
| --- | --- | --- |
| `Gate` | struct | Counts the mock requests in flight and can hold new ones back. `AppState.gate`. |
| `Gate::enter()` | async fn | Waits while the gate is paused, then counts the request in; the returned `Entered` guard, held until the answer is built, counts it out on drop. A request that only waits does not count as in flight. |
| `Gate::pause()` | fn | Holds new requests back until the returned `Paused` guard drops (on every path out of an Apply, including a panic). |
| `Gate::drained(timeout)` | async fn | Waits until nothing is inside, at most `timeout`; true when drained. |

The order that makes it safe: `enter` counts itself in first and reads the flag second; Apply sets the flag first and reads the count second, so a request is either seen by the drain or sees the pause.

`draft/apply.rs` also holds `apply(st, timeout)`, which builds the draft, pauses the gate, drains, swaps and resumes. The locks it takes, in order: `apply_lock` (one Apply at a time), the gate, then the `control` lock for the swap, under which the chaos state, the scenarios and the generation change together. `ApplyError` is `Invalid(Diagnostic)`, `Timeout` or `Internal`. The swap keeps chaos, runtime scenarios, leases and pins by name through `remap.rs`; scenario files that no longer resolve are flagged. The `control` state is cloned first and put back if the publish fails (the old generation stays, so its control state must), and the collections are emptied with the swap. `MAX_DRAFT_BODY` (about 6 MiB) is the body limit of the draft routes, looser than the 1 MiB text limit so a text of escapes is judged by the text limit.

`draft/remap.rs` keeps what still applies across an Apply:

| Item | Kind | What it does |
| --- | --- | --- |
| `remap_control(old, new, scenarios, control)` | fn | Rebuilds the `Control` state for the new generation under the control lock: live chaos and chaos leases through a name-based `KeyMap` (node by name, edge by its two node names, a replica only while it exists, an endpoint only while it has an entry node), the runtime scenarios (remapped like live chaos; the draft's definition wins on a name clash), the active scenarios the new set still defines (with their activation leases), and the live pins the API would still accept (not a synthesized `HEAD`, has an entry node, an `empty` pin still has a form). A `bandwidth_cap` the new edge cannot take is removed (`drop_stale_caps`). Returns a `Report`, or an error (the active stack could not be rebuilt) with `control` untouched. |
| `Report` | struct | `kept` (`{ live_chaos: n, active_scenarios, runtime_scenarios, pins: n }`) and `dropped` (`{ live_chaos: [labels], scenario_chaos: [labels], active_scenarios, pins }`), the shapes of the Apply answer. |

`Generation::build` takes a `lenient` flag for this: a scenario *file* that does not resolve against the new topology is left out and listed in `Generation.flagged` (a [`Flagged`](generation.md)) instead of failing the build; startup and `check` stay strict.

The routes are in [control.md](control.md) (`draft.rs`); the HTTP contract is in [docs/web/control-api.md](../../../web/control-api.md#draft).
