---
title: Install
description: Install Kaanal and start it.
sidebar:
  order: 1
---

Kaanal is one binary. Pick the way that suits you; each gives you the same `kaanal` command.

## Install

- **A release archive.** Download the one for your system (Linux x86_64, macOS arm64, Windows x86_64) from the
  [releases page](https://github.com/Naveen2070/kaanal/releases), check it against `SHA256SUMS`, and put `kaanal` on
  your PATH.
- **From crates.io** (Rust 1.100 or newer): `cargo install kaanal`.
- **With npm:** `npx kaanal` runs it without installing, or `npm install -g kaanal`.
- **From source:** build Studio once, then install.

  ```bash
  git clone https://github.com/Naveen2070/kaanal && cd kaanal
  (cd ui && pnpm install && pnpm build)
  cargo install --path crates/kaanal
  ```

  Without the Studio build the binary still runs; `/__kaanal/studio` then shows the status page.

:::note
The crates.io and npm packages are published with the 0.1.0 release. Until then, use a release archive or build from
source.
:::

Check it worked:

```bash
kaanal --version
```

```text
kaanal 0.1.0
```

## Run it

```bash
kaanal up                   # reads ./kaanal.yaml, serves on http://localhost:8080
```

`kaanal up` prints two URLs (and an `Auth:` line when the file has an `auth` node):

```text
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/studio?mode=admin#token=<64 hex chars>
Auth:  POST http://localhost:8080/login, JWKS http://localhost:8080/.well-known/jwks.json, 1 user
```

Point your frontend's `API_BASE_URL` at the mocks URL. Open the UI URL: it is [Studio](/docs/guides/simulate/studio/),
on its Admin tab when a project is loaded, where you watch requests, flip chaos and rewrite what a route answers. Only
one instance runs per project directory; a second `kaanal up` tells you it is already running. Ctrl+C stops it. For
scripts and CI, `kaanal up -d` starts it in the background and `kaanal down` stops it: see
[Lifecycle](/docs/guides/observe/lifecycle/).

**Log in from your frontend.** With an `auth` node, `POST /login` returns a signed JWT. Send
`Content-Type: application/json` (a `fetch` with `JSON.stringify` and no header sends `text/plain` and gets 415). See
[Auth](/docs/guides/mock/auth/).

No project yet? The [Quick start](/docs/start/quick-start/) gets you one in a minute.
