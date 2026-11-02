# Kaanal Studio (`/__kaanal/studio`)

Studio is Kaanal's graphical editor: a React single-page app that the binary serves from its own port. This page covers how it is served, how to start it and how to build it. The status page (`/__kaanal/ui`) stays as it is; see [web-ui.md](web-ui.md).

## Status and what is next

Studio can be used end to end today: build a system on the canvas (or load the starter), watch the cards and the top bar update after every edit, break things on purpose, read the log, the results and the charts, undo a mistake, then Apply or Save. What it does not do yet:

- It shows the numbers of a **finished** run, not a live stream (a later release adds live updates, pause, step and reset).
- If the file on disk changes while Studio is open, Save refuses and says so; the dialog offers **Overwrite the file** (your draft wins) and **Reload the file** (the file wins, your draft is dropped), and Copy my draft first if you want both.
- Not built yet: defining a scenario or saving live chaos as one from the page, a real request through the gateway, the replay animation, and live updates.
- Chaos acts on the system that is **applied**, while the canvas draws the **draft**: the Chaos panel says so when the draft has unapplied changes and offers **Apply the draft** on the spot; a component that is only in the draft stays blocked until it is applied.
- Captions on the canvas are kept in the browser for that project, not in the file: they are not saved with it or shared (see "Notes" below). **Planned (decided, not built):** move them into `kaanal.layout.json`, so a Save writes them and they travel with the project.
- Undo covers edits to the draft, not moving a node.
- It has been tested with an automated suite (including axe-core over the main components) but not yet by hand in every browser; expect rough edges and please report them.

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
- **Cards:** each node is a 236 px card, a step lighter than the panels, with a coloured top edge and a tinted icon badge in the hue of its kind (gateway violet, service blue, monolith teal, database cyan, cache orange, external API pink, auth green) and its name (the name is a button, so a keyboard can select it), a line of what it is set to in words ("Service · 1 copy × 32 slots · 25 ms"), a meter, and two figures, "Arriving" and "Slowest work under" (time spent working there, not waiting in line; shown as seconds above 1,000 ms). The meter is an 8 px track filled at the true busy share (green, amber from 70%, red from 90%) with the words above it in ink: "78% busy · up to 12 waiting" (the longest queue of the run, never a live count), "Idle", "Saturated · up to 40 waiting", or "No slot limit · never saturates". The words carry the state, colour only repeats it, and a screen reader hears "app, Service, 78 percent busy". While a run is pending the figures dim and say "updating". Select a card and its settings open in the **dock**, a panel floating at the bottom centre of the screen (never inside the card, so the card never changes size and nothing is clipped or shrunk by the canvas zoom): Time (how long one request takes once it has a slot; a log slider from 0.1 ms to 60 s and a box from 0 to 60000 ms, where 0 is allowed and says "0 ms never saturates"), Slots (how many requests one copy works on at once, 1 to 100000) and Copies (1 to 64), side by side. Databases and caches have no Time row (they price their work per operation), and a service time written as a distribution is shown as "edit in kaanal.yaml". While a slider moves only the line "Can finish up to 1,280/s · 32 at once ÷ 25 ms" follows it and the card's meter says "updating"; the change is sent when you let go (or release the key), a typed value on Enter or when you leave the box, and a value out of range puts the real one back. Escape or the close button deselects the card.
- **First run:** the loader (the mirage sun on a horizon, its reflection shimmering in the water, a quote that never repeats back to back, and a status line "Step 1 of 3 · Starting the simulator" to "Step 3 of 3 · Running the first simulation", no percentage) stays at least 10 s (the minimum load time, so it is seen rather than flashed; the setting `kaanal-min-load-ms` in the browser's storage, 0 to turn it off, a settings menu will own it) and until the draft has loaded and the first run has finished or been blocked (a draft with no `traffic:`, or one that does not build, opens the app with the reason shown), and gives up after 15 s so it can never loop. A draft that cannot be loaded shows the reason and a Retry on the loader. With reduced motion, or a hidden tab, there is no hold and it cuts straight to the app; a failure is never held. Then, for a draft with no components yet (a project that already has some gets no card, so the demo can never replace real work), a welcome card (Draw, Break, Run, Save) offers **Start blank**, **Try the demo** (replaces the draft with Studio's starter system, after any edit still on its way) and **Don't show again**; closing it starts a four-step tour (Add, Connect, Break, Save). Each step waits for the real action (a new component, a new connection, a run that saturates something, a Save) and then moves on by itself; Skip is always there, the note sits beside its target and never over it, keyboard focus goes to Skip and comes back to Run when the tour ends. The card is a real modal dialog: the rest of the page is inert while it is open. The card shows once per project (the browser remembers, so another browser shows it again); "Don't show again" is global and also skips the tour; the **?** in the top bar replays the tour whatever was chosen. Browsers that block storage simply show it every time.
- **Right rail:** above the tabs, one plain sentence about the selected component ("Handles up to 1,280 requests a second. It gets 1,000, so it is 78% busy and up to 12 wait."), announced politely when a run lands, never while a slider moves. Tabs **Controls / Log / Results / Live** (arrow keys move between them). The rail keeps one width (360 px) on every tab and stays beside the canvas, never over it. The button at its top (or the `]` key) folds it to a slim handle, the canvas refits to the room, and the choice is remembered; with no choice yet it starts folded below 1280 px.
  - **Controls:** name, slots, copies, service time, queue limit, timeout, a **Connect to…** list (the other way to draw an edge: only valid targets are offered), break it on purpose (the node, one instance or a link; it acts on the running system, so a component that is only in the draft is blocked, with the reason beside the buttons, until it is applied), what that works out to, a suggested fix, and Delete.
  - **Log:** a bounded sample of the newest run as a table (virtual time `mm:ss.mmm`, a level badge with its word, component, event in words, ms). Filter chips: All, Errors only, This component and one per endpoint (a filter the newest run lacks is dropped, and says so). A click on an event opens that request below the table as a waterfall; the wait in line is amber and one sentence says how much of the time was waiting, only when the request has `wait_ms`; a failed request lists its hops and the failure. Escape closes it. These are simulated trace events, not process output.
  - **Results:** the run number and "A what-if, not a prediction", warnings first, **What breaks first** (components ranked by peak busy share with "saturates at N req/s"; the first bar red, the others ink), the latency heatmap for one component (axis 0 s / middle / end, a table for screen readers), instances (visits/s, busy, wait), links, and errors by kind (zero prints as 0). Content waits behind a grey skeleton of the final size, so nothing jumps.
  - **Charts** (a floating panel at the bottom, closed until you open it, remembered): p99 latency, goodput and failures per window, one hue each, with a y scale and a text alternative.
  - **A picked wire** (click one, and no component is selected): its settings open in Controls. **Network latency** (a plain duration in ms, empty for none; a spread such as p50 and p99 is shown and edited in the file), **Bandwidth** (KB/s, empty for no limit, with a button to remove it), **Calls per request** (1 to 1000), **Share of requests** in percent (not offered on a wire out of a gateway or into a cache, where the config does not accept it), **Balance** (round robin or random, offered when the component it feeds has copies; it overrides that component's own), and **Delete wire**. Every value is checked against the loader's limits before it is sent.
  - **Nothing selected:** the three components nearest to breaking, then the **Traffic mix**: one bar and a row per endpoint of the first profile with its share, about how many requests a second that is, and its weight, which can be edited in place (a weight is relative: 4 and 1 is 80% and 20%).
  - **Live** (a fourth tab): what is switched on in the running system, apart from the file. **Scenarios** (named sets of failures and pins) turn on and off in a stack, with a lease of 30 seconds or 5 minutes if you want one; **Pins** make one endpoint answer as loading, empty, error, slow or success (a status for error and empty, a delay for all but success), with **Next** to step to the next state and **Remove**; **Sample a response** shows one body the mock would generate for an endpoint, its status and size. It needs an applied system and looks again every few seconds, since a lease can end on its own. Nothing here is written to the file.
- **Balance on a component:** with more than one copy, the Copies area of Controls has **Balance**: round robin (the default) or random. A weighted balance is shown but edited in the file.
- **Notes:** the **Add a note** button at the foot of the wide palette (or the command palette) pins a caption in the middle of the canvas. Type in it, drag it, delete it with its corner button. Notes belong to the project in this browser; they are not in `kaanal.yaml` or the layout file, so a Save does not write them and another browser does not see them. The plan is to store them in `kaanal.layout.json` beside the positions (a `notes` list), saved by Save, read again by Reload and kept when a component is renamed; until then they stay in the browser. At most 50 per project, 500 characters each.
- **Hints and status:** the foot of the wide palette says how to move around (drag the canvas to pan, scroll to zoom, drag from a card's right edge to another card to connect, Delete removes a picked wire, ? for the shortcuts). The line beside the charts toggle reads, for example, "3 components · 2 connections · re-run in 0.4 s", or "re-running..." while a run is going; before the first run it shows the simulated time instead, and the tooltip always gives both.
- **Help:** every jargon label (Time, Slots, Copies, p99, goodput, Breaks first, queue limit) has a 24 px **About** button that opens one sentence; Escape or a press outside closes it. Tooltips are only on icon buttons.
- **Notices:** every message about the state of things (a run that could not give numbers, a refused edit, a connection the draft would not take, a failed chaos change, an edit that cannot be undone, "Traffic updated") is a closeable toast at the top of the free space over the canvas, never a box on the canvas or a line in the top bar. Errors and warnings stay until you close them; information and success close themselves after six seconds. The same text is never shown twice, and a source's recovery closes its notice.
- **View controls:** a pill at the bottom, beside the details rail: zoom out, the zoom now, zoom in, fit the view and full screen (the whole page; the button says which way it will go).
- **Traffic builder:** the **Build traffic** button beside the offered load opens a dialog on the profile the slider drives (`peak`, or the first one; a blank draft gets `peak` at 100/s). One row per endpoint: method (GET, POST, PUT, PATCH, DELETE), path, weight in the mix, request and response size in KB, and the services behind the gateway: pick some, or **All services**. Several are called **in parallel** (the request waits for the slowest); a service the gateway has no edge to is connected when you apply, and the footer says which. An endpoint whose chain has other steps (a cache, say) shows "Custom chain, kept as written"; the inner chain of a service that stays is kept. Apply writes `endpoints:`, the profile's `journeys:` and the new edges as one edit (one undo); removing a row takes it out of the mix and leaves the route in the file. It is also in the command palette.
- **Command palette:** Ctrl/Cmd+K opens a modal list: add a component, run, apply, save, switch theme, replay the tour, fit the view. Type to filter, arrows to move, Enter to run, Escape to close.
- **Without dragging:** click a palette item to add it at the middle of the view; **Connect to…** draws an edge; arrow keys move the selected node (Shift for larger steps) and the position is saved a moment after the last press.
- **Undo and redo:** the two buttons in the top bar, Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z (not inside text fields). Up to 100 edits; the history is dropped if the draft changes from outside (a reload that finds a different revision, a server restart). An edit that cannot be undone is not recorded, and the bar says so.
- **Auto re-run:** every edit of the system, after a 600 ms pause, runs the simulator on the draft (never Apply). One run at a time; a busy server is waited out; a result from an older draft is thrown away; a draft that does not build keeps the last numbers dimmed with the reason. Values are checked before they are sent (whole numbers, ranges the loader accepts), and an edit that still leaves the draft unparseable is undone with the reason shown, so the canvas never goes blank. The **Apply** and **Save** buttons are the only things that leave the draft: Apply makes the draft the running system (the run loop never does), and shows what it kept and dropped; Save writes `kaanal.yaml` (and the positions file), asks first when comments would be lost, and on a conflict with the file on disk writes nothing and offers to copy your draft. The top bar is in groups with a divider between them (the title, Run with its status line, undo and redo, then the draft state with Apply and Save), one sheet at the top left as wide as its content with the key figures under it; theme and help are a small sheet at the top right. The draft state reads `Draft: applied · saved` or `Draft: not applied · not saved`, separate from the run state beside it ("Running...", "Up to date"). The **key figures** row under the top bar reads, left to right: **Breaks first** (the largest figure: the component that gives out at the lowest load, "db at 1,300 req/s", red only when it is saturated at the current load, "nothing saturates" in grey otherwise), **Offered load**, **System p99**, **Goodput**, **Errors** and **Dropped**. Offered load is a typed box and a logarithmic slider (10 to 15,000 requests per second; a number typed beyond the slider is accepted up to 1,000,000 and pins the thumb); it edits the first traffic profile's rate, commits when you let go of the slider or press Enter, puts the real value back when a typed one is refused, and goes through the same loop. A marker on the slider shows where the first component breaks. A figure with nothing behind it prints a grey dash, never zero.
- **Wires:** a wire is an orthogonal line with rounded elbows and an arrowhead at the component it feeds. It carries one **lane per copy** of that component, each lane in that copy's colour, drawn as a pulse of dashes that moves toward the arrow, faster and heavier as that copy takes more of the load (the copy's own figure when the run has one, otherwise the node's load shared evenly). A halo in amber or red shows under the wire when the component it feeds runs hot. The lanes stand still with reduced motion. Select a wire (click it) to see a delete button on it, or press Delete or Backspace; the Controls tab also lists each wire leaving the selected component with its own delete button. Deleting a wire is an edit like any other and can be undone. The view fits once on load and never zooms in past 90%; a component added later only moves the view, and only when it would land behind a panel.

### What can connect to what, and what a wire carries

A wire points from the caller to what it calls. The Studio offers, and the loader accepts, only these (rule R1.24):

| From                          | May call                                               |
| ----------------------------- | ------------------------------------------------------ |
| gateway                       | service, monolith, auth                                |
| service, monolith             | service, monolith, database, cache, external API, auth |
| auth                          | service, monolith, database, cache, external API       |
| database, cache, external API | nothing (they are leaves)                              |

The graph has no cycles, nothing calls the gateway, and a component calls another at most once per wire. Whether a wire is _allowed_ and whether it _carries traffic_ are different questions:

- The gateway runs its own time, then each auth component it is wired to, then calls the service that owns the route. It must have a wire to that service.
- **Auth is a check, not a forwarder.** The gateway's token check charges the auth component's own time only. Wires leaving auth are used by the login route alone (`POST /login` reads the user store, then any cache, service or external API behind it). Traffic to other routes never follows them.
- A service runs its own time, then the services it calls, then its cache for reads, then its databases.
- A request that fails at a step stops there: if auth rejects or fails it, the service is never called.

A wire dashed on the canvas, or marked "no traffic in the last run" in the Controls tab, carried nothing in the last run: its source had requests and its target got none. That says nothing is wrong with the wire, only that the route the traffic took did not use it.

## Look and feel

- **Theme:** the sun, moon and monitor button at the right of the top bar cycles system, light and dark, from the system setting by default. Neither theme uses pure white or pure black: light is warm paper (ground `#ece7de`, sheets `#f8f5ef`, cards `#fffdf9`), dark is a soft charcoal with a trace of violet (ground `#17161c`, sheets `#201f27`, cards `#282731`), so a long session is easy on the eyes. Blue stays the only action and data colour, and each kind of component and each copy lane has its own hue (`--cat-*`, `--lane-*`) (`<meta name="color-scheme">` and CSS `light-dark()` in `ui/src/look/theme/tokens.css`). A saved choice wins: a tiny same-origin script (`boot-<hash>.js`, one of the bundle's entries) applies it to `<html data-theme>` before the first paint, so there is no flash; blocked storage just means the system setting. The colour tokens are checked for WCAG contrast (text 4.5:1, non-text 3:1) in both themes by a test that reads the CSS.
- **Fonts:** Inter (text), JetBrains Mono (ids and traces) and a Noto Sans Tamil subset (the lockup), all OFL and served from the binary; nothing comes from a CDN. `ui/FONTS.md` lists files, licences and how the Tamil subset was cut.
- **Icons:** the mark (`ui/src/look/brand/Mark.tsx`, numbers in `geometry.mjs`), a 16 px favicon master and a 180 px touch icon (`node scripts/icons.mjs`, no image library).
- **Layout:** the canvas is the page: it fills the window with grid lines, and the top bar and key figures, the palette, the details rail, the dock and the charts float over it as sheets. At 1280 px and wider the palette is a list; from 768 to 1279 px it is an icon rail (each icon keeps its name); under 768 px the canvas is read-only: a list of the components, each with its state in words, and the reason. The top bar's three parts never overlap: the key figures wrap, the theme choice (system, light, dark as three buttons) stays at the right. Scrollbars are thin and compact everywhere.
- **Bundle checks:** `pnpm size` also fails on any source map or `sourceMappingURL` in `ui-dist`; fonts and icons are always files (never `data:` URIs, which the CSP refuses).

## Where the UI code lives

`ui/src` is grouped by role. A file's folder says what it is for; a test sits beside the file it tests.

| Folder      | What is in it                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------------- |
| `canvas/`   | The system drawn: component cards, wires, the control dock, the view and its keys                 |
| `data/`     | Talking to the server (`api/`) and the state the page keeps (`store/`: draft, run loop, history)  |
| `panels/`   | The floating work areas: the details rail (`details/`), the traffic builder, the charts band      |
| `shell/`    | The frame around the canvas: layout and responsive modes, the top cards, notices (toasts)         |
| `overlays/` | Things that open over the page: first-run welcome and tour, command palette, glossary, menu, help |
| `look/`     | How it looks: the brand mark and loader, theme tokens, fonts                                      |
| `kit/`      | Small shared parts: the slider and tabs, and the boundary for lazy chunks                         |
| `test/`     | Helpers the tests share (fake fetch, sample summaries)                                            |

A new feature goes in the folder of its role; a new menu entry is one line in `overlays/menu/MenuPanel.tsx`.

## Size budget

The bundle is part of every binary, and the first load is what a person waits for, so it has a budget that `pnpm size` (and CI) checks from the manifest. Lazy chunks (the Log and Results tabs, the charts band, the tour and the command palette) are not counted: they load when first used.

| Part                                               | Budget | Measured |
| -------------------------------------------------- | ------ | -------- |
| First-load JS + CSS (gzip, 1 kB = 1024 bytes)      | 350 kB | 202 kB   |
| First-load fonts (Inter and JetBrains Mono, WOFF2) | 100 kB | 87 kB    |

The numbers are guesses to revisit once measured on the redesigned UI.

## Release

The release workflow (`.github/workflows/release.yml`) builds the UI in its own `ui` job, hands `ui-dist` to every build job and fails before `cargo build` if `index.html` or `.vite/manifest.json` is missing, so a release never ships the fallback page.

To-do for a crates.io release (none exists yet; also listed in `RELEASE.md`): `cargo package` respects `.gitignore`, so the git-ignored `ui-dist/` would be left out of the published crate. Build the UI and make sure `ui-dist` is included before packaging. `Cargo.toml` deliberately has no `include = [...]` list (no crate here has one, and adding one would silently drop other files).

## Keys and notices

- **Wires:** click a wire or focus it and press Enter to select it, then Delete or Backspace removes it; the notice offers **Undo**. Connect-to and dragging a wire offer only what the validator accepts (a service never offers a gateway).
- **New components:** a click on the palette (or the command palette) puts the component in free space in the visible middle and selects it. A drop keeps the exact spot.
- **Arrow keys** nudge the selected component only while focus is on the canvas.
- **Command palette** (Ctrl+K) also jumps to a component by name ("Go to shop").
- **Notices** stay while the pointer or focus is on them; Undo and Redo clear the notice a bad edit raised and say "Undone" or "Redone". A blank draft says nothing about running until something is drawn.
- **Charts:** p99 is drawn as a moving average (it steps between histogram buckets); each chart has a time axis.
- **Traffic routes:** on a gateway's Controls tab each endpoint has the same service picker as the traffic builder: an **All services** box and one box per service. Several are called at the same time; a service the gateway has no wire to is wired when you pick it. A chain written by hand is left as it is.
- **System summary:** beside the charts button, a pill reads the draft's parts by sort (traffic, compute, data, external), the connection count and the simulated time of the last run. It stays whether the charts are open or closed.
- **Deleting a wire** also takes the calls that used it out of the endpoints' chains in the same edit (so the draft never says "gateway has no edge to service"), and the notice names the endpoints that stopped forwarding. Undo puts both back. Deleting a whole component is not cascaded yet.

## Credits

The canvas is drawn with [React Flow](https://reactflow.dev) (`@xyflow/react`, MIT). Its corner credit is hidden in the Studio; the credit is here instead.
