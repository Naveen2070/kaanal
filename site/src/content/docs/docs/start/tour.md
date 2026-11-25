---
title: "Try it: kaanal demo"
sidebar:
  order: 3
---

`kaanal demo` runs a small demo shop in your browser, backed by Kaanal, with a chaos bar to break it on purpose. It needs no project and no files of your own.

```
kaanal demo
```

```
Demo:  http://localhost:8080/#token=3f9c...
Seed:  7, locale: en-IN
Login: demo@example.com / demo
Try:   kaanal scenario activate slow-db --ttl 30s   (in another terminal, this folder)
Stop:  Ctrl-C, or kaanal down from this folder
```

Open the `Demo:` link to browse 24 products (priced in ₹), view details, and test cart + checkout behind login (`demo@example.com` / `demo`; a wrong password hits the real `401` path). Everything runs against Kaanal's mocked gateway, shop, auth, payment, DB, and cache nodes; orders and cart live in memory and clear on restart.

## The chaos bar

The bar sits at the bottom of the page, collapsed. Open it to:

- **Run a story (30 s TTL each):**
  - Infrastructure: `slow-db`, `cache-down`, `auth-errors`, `gateway-down`, `db-timeout`, `black-friday`, `flaky-network`, `cart-write-errors`, `brownout`, `meltdown`.
  - Payments: `payment-slow`, `payment-flaky` (40% `502`), `payment-down` (`503`), `payment-timeout` (`504` after 3 s), `payment-declined` (`402` via pin; failed payments never save an order).
- **Pin a state** on the product list: empty, error or loading, to see how the page handles each one.
- **Move the sliders**: latency and error rate for each node.
- **Reset all chaos** at once.

The page itself is never slowed or broken by chaos, so the bar stays usable even with the gateway down.

The stories are ordinary scenario files, so the command line drives the same instance. From a second terminal in the same folder:

```
kaanal scenario activate slow-db --ttl 30s
kaanal chaos set db --latency 800ms
kaanal logs -f
```

## The token in the link

The chaos bar talks to Kaanal's control API using a token passed after `#` (never sent to any server). The page saves it to `sessionStorage` (this tab only) and strips it from the address bar; without it, the shop still works with the bar hidden.

The link is the only place the token is printed. Do not paste it into chats or tickets.

## What it writes

`demo` writes the project to `.kaanal/demo/` on each start and keeps `run.json` and the lock in `.kaanal/` (only `.kaanal/` remains after stopping). Any existing `kaanal.yaml` in the folder is ignored with a note; if an instance is already running there, `demo` exits `64`.

## Taking the project with you: `--eject`

```
kaanal demo --eject [DIR]
```

Writes the same project to `DIR` (default `./kaanal-demo`) where you can see and edit it, and starts nothing: no server, no lock, no `.kaanal/`. It prints the folder and the next step:

```
Wrote the demo project (N files) to /home/you/kaanal-demo
Next:  cd kaanal-demo && kaanal up
```

The ejected project is a standard Kaanal project with a starter `peak` traffic profile (`100` rps for `30s`), so both `kaanal up` and `kaanal sim run -p peak` work out of the box.

| Topic            | Behaviour                                                                                                                                              |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `.gitignore`     | Adds `.kaanal/` (created if missing; non-fatal warning if unwritable).                                                                                 |
| Target directory | Creates new folders; existing folders must be empty unless `--force` is passed (which only overwrites demo files). Symlinks and junctions are refused. |
| Flags            | `--locale TAG` sets the config locale (unknown tags exit `64`). `--force` without `--eject`, or `--eject` with `--port`/`--open`, exits `64`.          |
| `--json`         | Prints `{ root, written, next }`.                                                                                                                      |

## Options

| Flag            | What it does                                                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `--port P`      | Port to serve on. The default is 8080; if it is taken, the next free one up to 8100 is used and the link shows it. `0` picks any free port. |
| `--locale TAG`  | Locale tag written into the demo's config, for example `de-DE`. Any built-in pack works; an unknown tag is exit code 64 and lists the tags. |
| `--open`        | Also open the link in your default browser.                                                                                                 |
| `--eject [DIR]` | Write the project to `DIR` (default `./kaanal-demo`) and start nothing. Not with `--port` or `--open`.                                      |
| `--force`       | With `--eject`: write into a folder that is not empty, overwriting only the demo's own files.                                               |

The demo always listens on `127.0.0.1` only and runs in the foreground: there is no `--host` and no `-d`. With `--json`, `demo` prints one object (`url`, `host`, `port`, `pid`, `locale`, `seed`, `auth`, `warnings`) instead of the lines above; it never contains the token, which stays in `.kaanal/run.json`.

`--open` passes the link, token included, to your system's opener (`open`, `xdg-open` or `rundll32` on Windows), so for a moment it is in that process's command line.

## Stopping

Press `Ctrl-C`, or run `kaanal down` in the same folder. Either way Kaanal finishes in-flight requests (up to 5 seconds) and removes `.kaanal/run.json`. See [Lifecycle](/docs/guides/observe/lifecycle/).
