# Kaanal Studio (`/__kaanal/studio`)

Studio is Kaanal's graphical editor: a React single-page app that the binary serves from its own port. This page covers how it is served, how to start it and how to build it. The status page (`/__kaanal/ui`) stays as it is; see [web-ui.md](web-ui.md).

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

## What the page does

- **Canvas:** drag a component from the palette (or click it), draw a line between two to connect them; the server checks every connection and a refused one is taken back with its reason.
- **Cards:** each node shows its busy bar (with the peak as a tick), a queue-depth sparkline, the longest queue, and "saturates near N req/s" (requests, not visits). A saturated card turns red and says so in words. Instances can be edited on the card, and service time on the kinds that have one (not databases or caches, which price their work per operation).
- **Details panel:** select a component for its Controls tab: name, slots, instances, service time, queue limit, timeout, break it on purpose (the node, one instance or a link; it acts on the running system, so a component that is only in the draft is blocked with a reason until it is applied, and setting chaos starts a run), what that works out to, a suggested fix, and Delete. Log and Results arrive in a later part.
- **Auto re-run:** every edit of the system, after a 600 ms pause, runs the simulator on the draft (never Apply). One run at a time; a busy server is waited out; a result from an older draft is thrown away; a draft that does not build keeps the last numbers dimmed with the reason. Values are checked before they are sent (whole numbers, ranges the loader accepts), and an edit that still leaves the draft unparseable is undone with the reason shown, so the canvas never goes blank. The top bar's offered-load slider edits the first traffic profile's rate and goes through the same loop.

## Size budget

The bundle is part of every binary, so it has a budget (a plan call, checked by hand for now).

| Part            | Budget (vite's kB = 1000 bytes) | Measured after Part C (cards, panel, loop) |
| --------------- | ------------------------------- | ------------------------------------------ |
| `app.js`        | 650 kB minified                 | 462 kB                                     |
| `app.js` (gzip) | 200 kB                          | 149 kB                                     |
| `app.css`       | 60 kB                           | 30 kB                                      |

The first budget (450 / 140 / 40) was set for a bare shell; the canvas library and React Compiler used it up in Part B, so it was raised once, with room for the panel, log and results (R1.97). Re-measure at the end of each part and when a dependency is added.

## Release

The release workflow (`.github/workflows/release.yml`) builds the UI in its own `ui` job, hands `ui-dist` to every build job and fails before `cargo build` if `index.html`, `app.js` or `app.css` is missing, so a release never ships the fallback page.

To-do for a crates.io release (none exists yet; also listed in `RELEASE.md`): `cargo package` respects `.gitignore`, so the git-ignored `ui-dist/` would be left out of the published crate. Build the UI and make sure `ui-dist` is included before packaging. `Cargo.toml` deliberately has no `include = [...]` list (no crate here has one, and adding one would silently drop other files).
