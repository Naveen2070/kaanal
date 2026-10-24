# Try it: `kaanal demo`

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

Open the `Demo:` link. The shop lists 24 products priced in rupees (₹), shows a product page, and has a cart and a checkout behind a login (`demo@example.com`, password `demo`; a wrong password shows the real 401 path). Checkout pays with a pre-filled test card through a simulated payment service, then shows the order, and an Orders page lists them. Everything it calls is mocked by Kaanal, with believable latency from a gateway, a shop service, an auth service, a payment service, a database and a cache. Orders, like the cart, live in memory and are gone on restart.

## The chaos bar

The bar sits at the bottom of the page, collapsed. Open it to:

- **Run a story**: a named scenario such as `slow-db` (the database answers slowly), `cache-down`, `auth-errors`, `gateway-down`, `db-timeout`, `black-friday`, `flaky-network`, `cart-write-errors`, `brownout`, `meltdown`, or one of the payment stories: `payment-slow`, `payment-flaky` (a 502 on four payments in ten), `payment-down` (503), `payment-timeout` (a hung call that gives up with a 504 after 3 s) and `payment-declined` (every card refused with a 402; a pin, because chaos never produces a 4xx). A failed payment never stores an order. Each one turns itself off after 30 seconds, and the bar shows the time left.
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

The chaos bar talks to Kaanal's control API, which needs the instance's token. The link carries it after `#`, a part of the URL that the browser never sends to any server. The page reads it, keeps it in `sessionStorage` (this tab only, gone when you close it), and removes it from the address bar. Without the token the bar hides itself and the shop still works; open the printed link again to get it back.

The link is the only place the token is printed. Do not paste it into chats or tickets.

## What it writes

`demo` writes the project to `.kaanal/demo/` in the current folder, rewriting it on every start, and keeps its state (`run.json`, the lock) in `.kaanal/` as `kaanal up` does. Nothing else in the folder is touched. After it stops, only `.kaanal/` is left; delete it when you are done.

A `kaanal.yaml` in the folder is ignored (`demo` prints a note), so you can try the demo inside a project. If a Kaanal instance is already running in the folder, `demo` stops with exit code 64 and changes nothing.

## Taking the project with you: `--eject`

```
kaanal demo --eject [DIR]
```

Writes the same project to `DIR` (default `./kaanal-demo`) where you can see and edit it, and starts nothing: no server, no lock, no `.kaanal/`. It prints the folder and the next step:

```
Wrote the demo project (N files) to /home/you/kaanal-demo
Next:  cd kaanal-demo && kaanal up
```

The ejected project is an ordinary Kaanal project: `kaanal up` serves the same storefront and the same stories, with no demo-only code path. `--eject` works while a demo is running.

- `.kaanal/` is added to the folder's `.gitignore` (created if missing, appended to otherwise), because `kaanal up` keeps the token and signing key there. A `.gitignore` that cannot be updated is a warning, not an error.
- A new folder is created. An existing folder must be empty; otherwise `--eject` stops (exit code 64) and changes nothing.
- `--force` writes into a folder that is not empty. It overwrites only the demo's own files and leaves every other file alone.
- `--locale TAG` sets the project's `locale:` as it does for the demo; an unknown tag is exit code 64 and creates nothing.
- A folder (or a file inside it) that is a symbolic link or junction is refused.
- `--force` without `--eject`, and `--eject` with `--port` or `--open`, are exit code 64.
- With `--json`: one object with `root`, `written` (the files, relative to `root`) and `next`.

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

Press `Ctrl-C`, or run `kaanal down` in the same folder. Either way Kaanal finishes in-flight requests (up to 5 seconds) and removes `.kaanal/run.json`. See [lifecycle.md](lifecycle.md).
