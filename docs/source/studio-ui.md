# Studio UI: building and layout

Contributor notes moved out of the public docs (they were in `https://kaanal.naveenr.in/docs/guides/simulate/studio/`). How the Studio frontend in `ui/` is built, embedded, developed and sized.

## The bundle is built apart from the binary

The frontend lives in `ui/` (Vite+, React, TypeScript). `cargo build` never runs Node or pnpm. `crates/kaanal-server/build.rs` looks for `crates/kaanal-server/ui-dist/.vite/manifest.json` (written by the UI build) and embeds exactly the files that manifest references, under `assets/`:

- The manifest exists: those files, with their `.br` and `.gz` siblings when present, are compiled into the binary (`include_bytes!`) and served as above. A file the manifest names that is missing fails the build, so a half-built bundle never ships.
- No manifest: `cargo build` prints a warning, and `/__kaanal/studio` serves the same page as `/__kaanal/ui` (the status page, with a notice that the build has no Studio). Every Rust build and test works without Node.

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
