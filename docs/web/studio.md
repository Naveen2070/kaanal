# Kaanal Studio (`/__kaanal/studio`)

Studio is Kaanal's graphical editor: a React single-page app that the binary serves from its own port. This page covers how it is served, how to start it and how to build it. The status page (`/__kaanal/ui`) stays as it is; see [web-ui.md](web-ui.md).

## Status and what is next

Studio can be used end to end today: build a system on the canvas (or load the starter), watch the cards and the top bar update after every edit, break things on purpose, read the log and the results, then Apply or Save. What it does not do yet:

- It shows the numbers of a **finished** run, not a live stream (a later release adds live updates, pause, step and reset).
- If the file on disk changes while Studio is open, Save refuses and the only recovery is copy, restart, paste (a reload button is planned).
- Not built yet: editing an edge (latency, bandwidth, probability), scenarios, pins and the traffic mix as panels, sample requests, a "saved to disk" indicator, and a visual design pass.
- Chaos acts on the system that is **applied**, while the canvas draws the **draft**: apply first to break something you just added.
- It has been tested with an automated suite but not yet by hand in every browser; expect rough edges and please report them.

## Start it

```
kaanal studio
```

Starts a blank instance (like `kaanal up --blank`) and opens your browser on `http://localhost:<port>/__kaanal/studio#token=<64 hex chars>`. `--no-open` only prints the URL. `kaanal up` keeps printing the status page URL.

The token rides in the URL **fragment**, which browsers never send to a server. Studio reads it, keeps it in `sessionStorage` for that tab and removes it from the address bar.

## What is served, and without a token

| Route                                 | Answer                                                                      | Token |
| ------------------------------------- | --------------------------------------------------------------------------- | ----- |
| `GET /__kaanal/studio`                | The bundle's `index.html`, or the status page when the binary has no bundle | No    |
| `GET /__kaanal/studio/assets/app.js`  | The script (404 when the binary has no bundle)                              | No    |
| `GET /__kaanal/studio/assets/app.css` | The style sheet (404 when the binary has no bundle)                         | No    |

These three exact paths are exempt from the token like `/__kaanal/ui` and `/__kaanal/health`, because the page has to load before it can read the token. Every API route behind them still needs the bearer token and a loopback `Host`.

## The bundle is built apart from the binary

The frontend lives in `ui/` (Vite+, React, TypeScript). `cargo build` never runs Node or pnpm. `crates/kaanal-server/build.rs` only looks for `crates/kaanal-server/ui-dist/index.html`, `app.js` and `app.css`:

- All three exist: they are compiled into the binary (`include_bytes!`) and `/__kaanal/studio` serves them.
- Any is missing: `cargo build` prints a warning, and `/__kaanal/studio` serves the same page as `/__kaanal/ui`. Every Rust build and test works without Node.

`ui-dist/` is git-ignored (except its `.gitkeep`); the bundle is never committed.

## Build the UI

Needs pnpm 12.16.1 (pinned in `ui/package.json`; pnpm downloads that version itself) and Node 26+ (the current LTS): pnpm stops with an error on an older Node. `devEngines.runtime` also lists Bun 1.4+, which only warns when the installed Bun is older or absent. pnpm checks every entry of that list, so it cannot mean "Bun, else Node"; the tools run on Node either way (`vp` is a Node program). The Vite+ CLI (`vp`) comes from the pinned dev dependency.

```
cd ui
pnpm install    # dependencies (pnpm-lock.yaml)
pnpm check      # format, lint, type check (vp check)
pnpm test       # Vitest, once (vp test run)
pnpm build      # writes app.js, app.css and index.html into crates/kaanal-server/ui-dist/
cd ..
cargo build     # picks the bundle up (build.rs re-runs when ui-dist changes)
```

File names are fixed (no hashes) because the server embeds exactly these three files. CI runs the same steps in its `ui` job and keeps `ui-dist` as the `studio-ui-dist` artifact.

## Develop against a running server

Editing `ui/src` and reloading `kaanal studio` needs a rebuild every time. For real iteration, run the UI from Vite's dev server (hot reload) while a real `kaanal` serves the API:

```
node scripts/studio-dev.mjs        # from the repo root
pnpm dev:studio                    # the same, from ui/
```

One command builds `kaanal`, starts a blank instance on port 7777, starts `vp dev` on 5173 with `/__kaanal` proxied to it, and prints one URL that already carries the control token:

```
Studio dev is up.
  Open:   http://localhost:5173/#token=<64 hex>
  API:    kaanal on 127.0.0.1:7777 (blank; Save writes <scratch>/kaanal.yaml)
```

Ctrl+C stops both. Options: `--no-build` (use the binary already in `target/debug`), `--fresh` (wipe the scratch project), `--dir PATH`, `--port N`, `--ui-port N`.

The scratch project defaults to `<tmp>/kaanal-studio-dev` and is **kept between runs**, so a system you drew is still there next time. `--fresh` wipes it. It is never your working directory: a blank start refuses to load an existing `kaanal.yaml`, and Save replaces that file, so pointing it at a real project would silently overwrite it.

Two details that are easy to get wrong, both already handled in `ui/vite.config.ts`:

- **The proxy sets `changeOrigin: true`.** The control guard (`kaanal-server/src/control/guard.rs`) rejects a `Host` that is not loopback **with the bound port**, so without it every API call gets a confusing `403 bad Host header`. The port comes from the `KAANAL_DEV_PORT` environment variable, which the runner sets, so the two cannot disagree.
- **`base` is `/` in dev and `/__kaanal/studio/assets/` only for the build.** Vite's own asset requests go through the same `/__kaanal` prefix the API uses; if the base applied in dev, they would be proxied to the server and answered by the *embedded* bundle — you would be editing code that never loads. Note this keys off Vite's `command` (`serve`/`build`), not its `mode` (`development`/`production`).

This page is not the embedded bundle: it has no CSP, and it is served by Vite rather than by the three exempt routes. It is for iterating on markup and logic — the hand acceptance pass still belongs on `kaanal studio`'s embedded page, which is the artifact under test.

## What the page does

- **Canvas:** drag a component from the palette (or click it), draw a line between two to connect them; the server checks every connection and a refused one is taken back with its reason.
- **Cards:** each node shows its busy bar (with the peak as a tick), a queue-depth sparkline, the longest queue, and "saturates near N req/s" (requests, not visits). A saturated card turns red and says so in words. Instances can be edited on the card, and service time on the kinds that have one (not databases or caches, which price their work per operation).
- **First run:** a loading page (a rotating quote and a small bar loader, no counter) stays until the draft has loaded and at least 0.6 s. Then, for a draft with no components yet (a project that already has some gets no card, so the demo can never replace real work), a welcome card (Draw, Break, Run, Save) offers **Start blank**, **Try the demo** (replaces the draft with Studio's starter system, after any edit still on its way) and **Don't show again**; closing it starts a three-stop tour (palette, panel, Run) that puts keyboard focus on Next and gives it back to Run when it ends. The card is a real modal dialog: the rest of the page is inert while it is open. The card shows once per project (the browser remembers, so another browser shows it again); "Don't show again" is global and also skips the tour; the **?** in the top bar replays the tour whatever was chosen. Browsers that block storage simply show it every time.
- **Details panel:** select a component for its Controls tab: name, slots, instances, service time, queue limit, timeout, break it on purpose (the node, one instance or a link; it acts on the running system, so a component that is only in the draft is blocked with a reason until it is applied, and setting chaos starts a run), what that works out to, a suggested fix, and Delete. The **Log** tab pages through a bounded sample of the newest run (errors only, instance, endpoint, this component; a click opens that request hop by hop as a waterfall); its times are virtual, simulated trace events, not process output. The **Results** tab shows the report label and warnings, a latency heatmap for one component (time across, slower hops higher, with a table for screen readers), instances, links, errors by kind and the order things break in.
- **Auto re-run:** every edit of the system, after a 600 ms pause, runs the simulator on the draft (never Apply). One run at a time; a busy server is waited out; a result from an older draft is thrown away; a draft that does not build keeps the last numbers dimmed with the reason. Values are checked before they are sent (whole numbers, ranges the loader accepts), and an edit that still leaves the draft unparseable is undone with the reason shown, so the canvas never goes blank. The **Apply** and **Save** buttons are the only things that leave the draft: Apply makes the draft the running system (the run loop never does), and shows what it kept and dropped; Save writes `kaanal.yaml` (and the positions file), asks first when comments would be lost, and on a conflict with the file on disk writes nothing and offers to copy your draft. The top bar says whether the draft is applied. The top bar's offered-load slider edits the first traffic profile's rate and goes through the same loop.

## Size budget

The bundle is part of every binary, so it has a budget (a plan call, checked by hand for now).

| Part            | Budget (vite's kB = 1000 bytes) | Measured after Part E (log, results, save) |
| --------------- | ------------------------------- | ------------------------------------------ |
| `app.js`        | 650 kB minified                 | 493 kB                                     |
| `app.js` (gzip) | 200 kB                          | 159 kB                                     |
| `app.css`       | 60 kB                           | 35 kB                                      |

The first budget (450 / 140 / 40) was set for a bare shell; the canvas library and React Compiler used it up in Part B, so it was raised once, with room for the panel, log and results (R1.97). Re-measure at the end of each part and when a dependency is added.

## Release

The release workflow (`.github/workflows/release.yml`) builds the UI in its own `ui` job, hands `ui-dist` to every build job and fails before `cargo build` if `index.html`, `app.js` or `app.css` is missing, so a release never ships the fallback page.

To-do for a crates.io release (none exists yet; also listed in `RELEASE.md`): `cargo package` respects `.gitignore`, so the git-ignored `ui-dist/` would be left out of the published crate. Build the UI and make sure `ui-dist` is included before packaging. `Cargo.toml` deliberately has no `include = [...]` list (no crate here has one, and adding one would silently drop other files).
