---
title: "Studio"
sidebar:
  order: 1
---

Studio is Kaanal's graphical editor: a React single-page app served from the binary's own port. For the token-less health view (`/__kaanal/ui`), see [the status page](/docs/guides/observe/status-page/).

## Status and what is next

Studio works end to end today: build a system on the canvas (or load a starter), watch cards and the top bar update after every edit, break things on purpose, read the log/results/charts, undo mistakes, then Apply or Save.

- **Current limits**:
  - Batch shows a **finished** run rather than a live stream; **Live** mode runs continuous traffic on the wall clock.
  - Not built yet: defining a scenario from scratch on the page (**Save live** is built), a real request through the gateway, replay animation, and live updates on Batch.
  - Chaos acts on the **applied** system while the canvas edits the **draft**; the Chaos panel offers **Apply the draft** on the spot and blocks draft-only components until applied.
  - Undo covers draft edits, not moving a node.
  - Tested with an automated suite (including axe-core), though by-hand testing across all browsers is still in progress.
- **File conflicts on Save**: if `kaanal.yaml` changed on disk while Studio was open, Save refuses and offers **Overwrite the file**, **Reload the file**, or **Copy my draft**.
- **Canvas notes**: captions are saved in `kaanal.layout.json` beside positions, so Save writes them with the project.
- **Network model**: first-order cut-through sharing (`bytes` over `min(sender out, wire, receiver in)`), shared fairly across transfers on each pipe. Connection set-up, streamed responses, and packet loss are configurable (see [Resources](/docs/reference/resources/)).
- **Units**: every time, size, and rate box has a unit selector (`ms`, `s`, `min`; `B`, `KB`, `MB`, `GB`; `KB/s`, `MB/s`, `GB/s`). Files store ms, bytes, and bytes/s; untouched rate text (`250KB/s`) is kept on Save. Long times read as `4.2 min` or `15 h`.

## Start it

```bash
kaanal studio
```

Starts a blank instance (`kaanal up --blank`) and opens `http://localhost:<port>/__kaanal/studio#token=<64 hex chars>` (`--no-open` prints the URL only). `kaanal up` also prints the Studio link with `?mode=admin` when a project is loaded.

The token rides in the URL **fragment** (never sent to a server); Studio saves it in `sessionStorage` for that tab and strips it from the address bar.

## What is served, and without a token

| Route                                | Answer                                                                                | Token |
| ------------------------------------ | ------------------------------------------------------------------------------------- | ----- |
| `GET /__kaanal/studio`               | The bundle's `index.html`, or the status page if no bundle is embedded                | No    |
| `GET /__kaanal/studio/assets/<name>` | Built asset (script, CSS, font, icon) by exact Vite manifest name; `404` if unbundled | No    |

Only manifest-listed files for `GET`/`HEAD` with no query string are served without a token; everything else under `/__kaanal/studio/` requires the bearer token and loopback `Host` (`401` without, `404` with). Assets are served with `nosniff`, `immutable` caching for hashed names (`no-cache` otherwise), `Vary: Accept-Encoding`, per-encoding `ETag` (`304`), and pre-compressed Brotli/gzip (honouring `q=0`, no range requests).

## What the page does

### Canvas & cards

- **Canvas**: drag a component from the palette (or click it) and draw wires between cards; refused connections revert with the validator's reason.
- **Cards:** `236 px` cards with a kind-colored icon badge, a focusable name button, a one-line summary (`Service · 1 copy × 32 slots · 25 ms`), and an `8 px` busy meter (`green` / `amber >= 70%` / `red >= 90%`).
  - Shows **Arriving**, **Turned away** (when non-zero), and **Slowest work under** (with help popovers); figures dim to `"updating"` while a run is pending.
- **Bottom dock**: selecting a card opens **Time** (`0.1 ms`–`60 s` log slider + `0`–`60000 ms` input; omitted on databases/caches; spreads show `"edit in kaanal.yaml"`), **Slots** (`1`–`100000`), and **Copies** (`1`–`64`). Sliders preview live capacity (`Can finish up to 1,280/s · 32 at once ÷ 25 ms`) and commit on release; `Escape` or Close deselects.

### First run & tour

- **Loader**: shows the mirage sun, a non-repeating quote, and `"Step 1 of 3"`–`"Step 3 of 3"` for at least `2.5 s` (`kaanal-min-load-ms` in storage, `0` to disable; `15 s` timeout; skipped under reduced motion or hidden tab). Failures show the reason and **Retry** immediately.
- **Welcome & tour**: blank drafts show a modal (**Start blank**, **Try the demo**, **Don't show again**) once per project, followed by a four-step interactive tour (**Add**, **Connect**, **Break**, **Save**) that advances automatically on real actions (replayable from **?** in the menu).

### Right rail

The rail keeps a resizable width (`320`–`640 px`, default `360 px`, drag splitter or arrow keys), folds with `]` (starts folded below `1280 px`), and shows one plain summary sentence above the tabs (**Controls / Log / Results / Live**). An empty canvas offers starter systems (**Shop**, **App + DB**, **Fan-out**, **A balanced tier**, **A file store**, and **Retry storm**).

- **Controls**: sticky header, run figures, **Why is it red?** / **Why does it look idle?** / **To lift the first limit**, **Quick setups** (Event loop, Thread per request, Small pool, Postgres default, Fail fast, Request limit), **Capacity** (service time, queue, balance, **Server style**), timeout, chaos actions (blocked on unapplied draft nodes), **Connect to…**, name, and Delete.
- **Log:** bounded sample of trace events grouped by request (`r-12`, endpoint, status, total time), indented `12 px` per call depth.
  - Rows show time, component, type badge (`Work`, `Network`, `Data`, `Cache hit/miss`, `Delay`, `Send`, `Reply`, `Rate limited`, `Queue full`, `Timed out`, `Breaker open`, `Failed`, `↻ Attempt N of M`), bar, and ms.
  - Includes **All / Errors only**, **Request** search, **Filters** (Endpoint, Component, Wire, Instance), **Load more**, and **Pop out** (`Escape` or **Bring back** returns it).
- **Results**: run number and `"A what-if, not a prediction"`, model hints (unseen chaos due to cache/unreached node; omitted `hold: call` / bandwidth), comparison with previous run, warnings, **What breaks first** (components and wires ranked by peak load), **Links**, latency/wire heatmap, **p99 target** SLO check, **Plan for a rate** (rough <70% busy scaling), instances, **Share this run** (Markdown copy/save), and errors by kind.
- **Charts**: collapsible bottom drawer showing p99 latency, goodput, and failures per window with y-scales and text alternatives.
- **Picked wire**: Network latency, Bandwidth (`KB/s`, `MB/s`, `GB/s`; `1 KB = 1024 B`), Request/Response sizes, Compression, Packet loss, Stream mode, TLS (`1.3` / `1.2` / none), Calls per connection, Retry policy (with budget), Timeout behavior (`abandon`), Circuit breaker (Resilience4j defaults), Calls per request, Share %, Balance override + `detect` delay, and Delete wire.
- **Nothing selected**: top 3 components closest to breaking, editable **Traffic mix** weights, and **Headers on every call** (`600 B` / `400 B` defaults).
- **Live tab**: toggle **Scenarios** in a stack (optional `30 s` / `5 min` lease), manage **Pins** (`loading`, `empty`, `error`, `slow`, `success` + **Next** / **Remove**), **Sample a response**, and **Save live** to `scenarios/NAME.yaml`.

### Component features & modes

- **Load balancer**: yellow split-arrow node; Settings offers **Spread** (`round-robin` / `random`; weighted in YAML) and **Notice a downed copy after**. Component/wire **Balance** also offers **Least connections** and **Power of two choices** (live queues in both runs and live mocks).
- **Storage**: slate archive node; **Flavor** (`s3`, `gcs`, `blob`), **First byte** (duration, `p50`/`p99`, or `mean`), **Throughput**, and **Request limit** (defaults to S3's `5,500/s` -> `503`). New nodes start with `25 ms` / `80 MB/s` placeholders.
- **CPUs**: folded **CPUs** group with **Cores** (`0.001`–`4096`); components can become **CPU bound** in **What breaks first** (honoured in live mocks too).
- **Request limit (all components)**: `rps` + `burst` (`429`, or `503` on storage); starter default `1,000/s`.
- **Queue & timeout defaults**: starters and new nodes default to `queue: 100` on slot limits and `timeout: 30s` on the gateway; Results warns when p99 exceeds the entry timeout (`30 s` default).
- **Live vs Batch mode**: **Live is the default**, streaming continuous traffic with animated queue dots and wire pulses (`60` max, paused under reduced motion or `60 s` hidden tab; closing Studio stops the generator after `5 s`). Settings changes (`cores`, `slots`, `queue`, `copies`, `bandwidth`, `rate_limit`, `time`, `balance`) apply immediately; topology edits wait for **Apply**. **Batch** runs seeded, repeatable simulations.
- **Notes**: **Add a note** pins captions stored in `kaanal.layout.json` (`<= 50` notes, `<= 500` chars each). Existing browser `localStorage` notes are imported once if the file has none.

### Editing, shortcuts & workflow

- **Auto re-run**: every draft edit pauses `600 ms` and re-runs the Batch simulator on the draft (never calls Apply). Invalid edits revert with a toast notice.
- **Key figures strip**: **Breaks first**, **Offered load** (`10`–`15,000 req/s` log slider, up to `1,000,000` typed), **System p99**, **Goodput** (notes `"queue still growing"` if saturated), **Errors**, and **Dropped**.
- **Traffic builder**: **Build traffic** edits endpoints, weights, request/response sizes, and parallel gateway→service edges in one undo step.
- **Steps editor**: on the route-owning component (`service`, `monolith`, `auth`), edits `chain:` steps (`call`, `cache`, `parallel`), per-step sizes/streaming/retries, and step order (`Alt+Up` / `Alt+Down`).
- **Wires**: orthogonal elbows with per-copy animated color lanes, amber/red halos on hot targets, and red tags (`4 KB/s · 140%`) on saturated links.
- **Undo / redo & palette**: `Ctrl/Cmd+Z` / `Ctrl/Cmd+Shift+Z` (up to `100` steps), `Ctrl/Cmd+K` command palette, arrow keys nudge selected nodes, and `24 px` **About** help buttons on jargon labels.

### What can connect to what, and what a wire carries

A wire points from the caller to what it calls. Studio offers, and the loader accepts, only these edges (rule R1.24):

| From                          | May call                                               |
| ----------------------------- | ------------------------------------------------------ |
| gateway                       | service, monolith, auth                                |
| service, monolith             | service, monolith, database, cache, external API, auth |
| auth                          | service, monolith, database, cache, external API       |
| database, cache, external API | nothing (they are leaves)                              |

The graph has no cycles, nothing calls the gateway, and a component calls another at most once per wire:

- The gateway runs its own time, then each auth component it is wired to, then calls the service owning the route (it must have a wire to that service).
- **Auth is a check, not a forwarder.** The gateway's token check charges the auth component's own time only. Wires leaving auth are used by `POST /login` alone (reading the user store, then any cache, service, or external API behind it).
- A service runs its own time, then called services, then its cache for reads, then its databases.
- A failing step stops the chain there.

A dashed wire marked `"no traffic in the last run"` carried nothing because the traffic mix did not route over it.

## Admin mode

The mode switch has a third button, **Admin**: the live view with a real frontend in it. Mocked requests appear in live figures tagged as real (rings on the canvas), the Log tab shows them as a request feed, and **Edit response** rewrites what a route answers. See [Admin mode](/docs/guides/break/admin-mode/).

## Look and feel

- **Theme**: top-right button cycles `system`, `light` (warm paper `#ece7de`), and `dark` (charcoal `#17161c`), persisted via `boot-<hash>.js` before first paint with WCAG-checked contrast (`4.5:1` text, `3:1` non-text). Blue is the sole action hue; component kinds and copy lanes use `--cat-*` and `--lane-*`.
- **Fonts & icons**: self-hosted `Inter`, `JetBrains Mono`, and Noto Sans Tamil subset (`ui/FONTS.md`), plus the SVG mark (`Mark.tsx`), `16 px` favicon, and `180 px` touch icon. No CDN requests or `data:` URIs.
- **Responsive layout**: full-bleed canvas with floating sheets; wide palette becomes a `56 px` icon rail below `1400 px`, and a read-only component list below `768 px`.
- **Bundle checks**: `pnpm size` fails on source maps or inline `data:` URIs.

## Keys and notices

- **Wires**: click or focus + `Enter` to select, `Delete`/`Backspace` to remove (also strips dependent calls from endpoint chains in one undoable edit).
- **Palette & nudging**: clicking a palette item places it in free visible space; arrow keys nudge while the canvas is focused; `Ctrl+K` jumps to components by name.
- **Notices & charts**: notices persist while hovered/focused and clear on Undo/Redo; p99 charts draw a moving average over time; the system summary pill beside Charts stays visible.

## Credits

The canvas is drawn with [React Flow](https://reactflow.dev) (`@xyflow/react`, MIT). Its corner credit is hidden in Studio and credited here instead.
