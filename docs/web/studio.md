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

| Route                                | Answer                                                                                                | Token |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------- | ----- |
| `GET /__kaanal/studio`               | The bundle's `index.html`, or the status page when the binary has no bundle                           | No    |
| `GET /__kaanal/studio/assets/<name>` | One built file (script, style sheet, font, icon) by its exact name; 404 when the binary has no bundle | No    |

Only a file that is in the binary's table is served without a token, and only for `GET` and `HEAD` with no query string. The table is made at build time from the Vite manifest, so no file name is ever read from disk at run time. Anything else under `/__kaanal/studio/` (another method, a missing file, an encoded or oddly cased path, `..`) needs the bearer token like the rest of the API: 401 without it, 404 with it. Every API route behind the page still needs the bearer token and a loopback `Host`. The page and the assets are exempt only because the page has to load before it can read the token.

Files are sent with the right `Content-Type`, `nosniff`, a long `immutable` cache lifetime for names that carry a content hash (and `no-cache` for any that do not), `Vary: Accept-Encoding`, a strong per-encoding `ETag` (`If-None-Match` answers 304), and pre-compressed Brotli or gzip when the client accepts it (`q=0` is honoured). There are no range requests.

## The bundle is built apart from the binary

The frontend lives in `ui/` (Vite+, React, TypeScript). `cargo build` never runs Node or pnpm. `crates/kaanal-server/build.rs` looks for `crates/kaanal-server/ui-dist/.vite/manifest.json` (written by the UI build) and embeds exactly the files that manifest references, under `assets/`:

- The manifest exists: those files, with their `.br` and `.gz` siblings when present, are compiled into the binary (`include_bytes!`) and served as above. A file the manifest names that is missing fails the build, so a half-built bundle never ships.
- No manifest: `cargo build` prints a warning, and `/__kaanal/studio` serves the same page as `/__kaanal/ui`. Every Rust build and test works without Node.

`index.html`, dot files, source maps and the manifest itself are never served from the assets route.

`ui-dist/` is git-ignored (except its `.gitkeep`); the bundle is never committed.

## Build the UI

Needs pnpm 12.16.1 (pinned in `ui/package.json`; pnpm downloads that version itself) and Node 26+ (the current LTS): pnpm stops with an error on an older Node. `devEngines.runtime` also lists Bun 1.4+, which only warns when the installed Bun is older or absent. pnpm checks every entry of that list, so it cannot mean "Bun, else Node"; the tools run on Node either way (`vp` is a Node program). The Vite+ CLI (`vp`) comes from the pinned dev dependency.

```
cd ui
pnpm install    # dependencies (pnpm-lock.yaml)
pnpm check      # format, lint, type check (vp check)
pnpm test       # Vitest, once (vp test run)
pnpm build      # writes index.html, assets/ (hashed, code-split) and .vite/manifest.json into crates/kaanal-server/ui-dist/, then precompresses
cd ..
cargo build     # picks the bundle up (build.rs re-runs when ui-dist changes)
```

File names carry a content hash (so a browser can cache them for a year) and the build empties `ui-dist/` first, so an old file never ends up in a binary. `pnpm size` checks the first-load budget below. CI runs the same steps in its `ui` job and keeps `ui-dist` as the `studio-ui-dist` artifact; the artifact step keeps hidden files, because the manifest lives in `ui-dist/.vite/` and a binary built without it would serve the fallback page.

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
- **`base` is `/` in dev and `/__kaanal/studio/assets/` only for the build.** Vite's own asset requests go through the same `/__kaanal` prefix the API uses; if the base applied in dev, they would be proxied to the server and answered by the _embedded_ bundle — you would be editing code that never loads. Note this keys off Vite's `command` (`serve`/`build`), not its `mode` (`development`/`production`).

This page is not the embedded bundle: it has no CSP, and it is served by Vite rather than by the three exempt routes. It is for iterating on markup and logic — the hand acceptance pass still belongs on `kaanal studio`'s embedded page, which is the artifact under test.

## What the page does

- **Canvas:** drag a component from the palette (or click it), draw a line between two to connect them; the server checks every connection and a refused one is taken back with its reason.
- **Cards:** each node is a 236 px card: a grey kind icon and its name (the name is a button, so a keyboard can select it), a line of what it is set to in words ("Service · 1 copy × 32 slots · 25 ms"), a meter, and two figures, "Arriving" and "Slowest requests under". The meter is an 8 px track filled at the true busy share (green, amber from 70%, red from 90%) with the words above it in ink: "78% busy · up to 12 waiting" (the longest queue of the run, never a live count), "Idle", "Saturated · up to 40 waiting", or "No slot limit · never saturates". The words carry the state, colour only repeats it, and a screen reader hears "app, Service, 78 percent busy". While a run is pending the figures dim and say "updating". Instances can be edited on the card, and service time on the kinds that have one (not databases or caches, which price their work per operation).
- **First run:** the loader (the mirage sun on a horizon, its reflection shimmering in the water, a quote that never repeats back to back, and a status line "Step 1 of 3 · Starting the simulator" to "Step 3 of 3 · Running the first simulation", no percentage) stays at least 10 s (the minimum load time, so it is seen rather than flashed; the setting `kaanal-min-load-ms` in the browser's storage, 0 to turn it off, a settings menu will own it) and until the draft has loaded and the first run has finished or been blocked (a draft with no `traffic:`, or one that does not build, opens the app with the reason shown), and gives up after 15 s so it can never loop. A draft that cannot be loaded shows the reason and a Retry on the loader. With reduced motion, or a hidden tab, there is no hold and it cuts straight to the app; a failure is never held. Then, for a draft with no components yet (a project that already has some gets no card, so the demo can never replace real work), a welcome card (Draw, Break, Run, Save) offers **Start blank**, **Try the demo** (replaces the draft with Studio's starter system, after any edit still on its way) and **Don't show again**; closing it starts a three-stop tour (palette, panel, Run) that puts keyboard focus on Next and gives it back to Run when it ends. The card is a real modal dialog: the rest of the page is inert while it is open. The card shows once per project (the browser remembers, so another browser shows it again); "Don't show again" is global and also skips the tour; the **?** in the top bar replays the tour whatever was chosen. Browsers that block storage simply show it every time.
- **Details panel:** select a component for its Controls tab: name, slots, instances, service time, queue limit, timeout, break it on purpose (the node, one instance or a link; it acts on the running system, so a component that is only in the draft is blocked with a reason until it is applied, and setting chaos starts a run), what that works out to, a suggested fix, and Delete. The **Log** tab pages through a bounded sample of the newest run (errors only, instance, endpoint, this component; a click opens that request hop by hop as a waterfall); its times are virtual, simulated trace events, not process output. The **Results** tab shows the report label and warnings, a latency heatmap for one component (time across, slower hops higher, with a table for screen readers), instances, links, errors by kind and the order things break in.
- **Auto re-run:** every edit of the system, after a 600 ms pause, runs the simulator on the draft (never Apply). One run at a time; a busy server is waited out; a result from an older draft is thrown away; a draft that does not build keeps the last numbers dimmed with the reason. Values are checked before they are sent (whole numbers, ranges the loader accepts), and an edit that still leaves the draft unparseable is undone with the reason shown, so the canvas never goes blank. The **Apply** and **Save** buttons are the only things that leave the draft: Apply makes the draft the running system (the run loop never does), and shows what it kept and dropped; Save writes `kaanal.yaml` (and the positions file), asks first when comments would be lost, and on a conflict with the file on disk writes nothing and offers to copy your draft. The top bar shows the draft's state as one line, `Draft: applied · saved` or `Draft: not applied · not saved`, separate from the run state beside it ("Running...", "Up to date"). The **key figures** row under the top bar reads, left to right: **Breaks first** (the largest figure: the component that gives out at the lowest load, "db at 1,300 req/s", red only when it is saturated at the current load, "nothing saturates" in grey otherwise), **Offered load**, **System p99**, **Goodput**, **Errors** and **Dropped**. Offered load is a typed box and a logarithmic slider (10 to 15,000 requests per second; a number typed beyond the slider is accepted up to 1,000,000 and pins the thumb); it edits the first traffic profile's rate, commits when you let go of the slider or press Enter, puts the real value back when a typed one is refused, and goes through the same loop. A marker on the slider shows where the first component breaks. A figure with nothing behind it prints a grey dash, never zero.
- **Edges:** an edge is an orthogonal line with rounded elbows that gets thicker with the requests it carries (1.5 to 4.5 px), takes the colour of its busier end (grey, amber from 70%, red from 90%), and shows a dash flowing along it, faster with load; the flow is off with reduced motion. The view fits itself on load and after a component is added.

## Look and feel

- **Theme:** the sun, moon and monitor button at the right of the top bar cycles system, light and dark. Light and dark, from the system setting by default. In light the window ground and the inset wells are a faint warm cream while the sheets stay white, and blue stays the only action and data colour; dark is neutral black (`<meta name="color-scheme">` and CSS `light-dark()` in `ui/src/theme/tokens.css`). A saved choice wins: a tiny same-origin script (`boot-<hash>.js`, one of the bundle's entries) applies it to `<html data-theme>` before the first paint, so there is no flash; blocked storage just means the system setting. The colour tokens are checked for WCAG contrast (text 4.5:1, non-text 3:1) in both themes by a test that reads the CSS.
- **Fonts:** Inter (text), JetBrains Mono (ids and traces) and a Noto Sans Tamil subset (the lockup), all OFL and served from the binary; nothing comes from a CDN. `ui/FONTS.md` lists files, licences and how the Tamil subset was cut.
- **Icons:** the mark (`ui/src/brand/Mark.tsx`, numbers in `geometry.mjs`), a 16 px favicon master and a 180 px touch icon (`node scripts/icons.mjs`, no image library).
- **Layout:** at 1280 px and wider the palette, canvas and right rail are three sheets; from 900 to 1279 px the palette is an icon rail (each icon keeps its name); under 900 px the canvas is read-only: a list of the components and the reason.
- **Bundle checks:** `pnpm size` also fails on any source map or `sourceMappingURL` in `ui-dist`; fonts and icons are always files (never `data:` URIs, which the CSP refuses).

## Size budget

The bundle is part of every binary, and the first load is what a person waits for, so it has a budget that `pnpm size` (and CI) checks from the manifest. Lazy chunks (the Log and Results tabs, charts, the tour and the command palette) are not counted: they load when first used.

| Part                                          | Budget | Measured |
| --------------------------------------------- | ------ | -------- |
| First-load JS + CSS (gzip, 1 kB = 1024 bytes) | 200 kB | 164 kB   |
| First-load fonts (Latin text face, WOFF2)     | 60 kB  | 47 kB    |

The numbers are guesses to revisit once measured on the redesigned UI.

## Release

The release workflow (`.github/workflows/release.yml`) builds the UI in its own `ui` job, hands `ui-dist` to every build job and fails before `cargo build` if `index.html` or `.vite/manifest.json` is missing, so a release never ships the fallback page.

To-do for a crates.io release (none exists yet; also listed in `RELEASE.md`): `cargo package` respects `.gitignore`, so the git-ignored `ui-dist/` would be left out of the published crate. Build the UI and make sure `ui-dist` is included before packaging. `Cargo.toml` deliberately has no `include = [...]` list (no crate here has one, and adding one would silently drop other files).
