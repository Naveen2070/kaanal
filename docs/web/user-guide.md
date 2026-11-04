# User guide

Kaanal serves a fake version of your backend from a single YAML file, with realistic data, realistic timing, and chaos you can flip live.

## Install & run

```bash
cargo install kaanal        # or grab a release binary
kaanal up                   # reads ./kaanal.yaml, serves on http://localhost:8080
```

`kaanal up` prints two URLs (and an `Auth:` line when the file has an `auth` node):

```
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/ui#token=<64 hex chars>
Auth:  POST http://localhost:8080/login, JWKS http://localhost:8080/.well-known/jwks.json, 1 user
```

Point your frontend's `API_BASE_URL` at the mocks URL. Open the UI URL to watch requests and flip chaos. Only one instance per project directory; a second `kaanal up` tells you it's already running. Ctrl+C stops it. For scripts and CI, `kaanal up -d` starts it in the background and `kaanal down` stops it: see [lifecycle.md](lifecycle.md).

**Log in from your frontend.** With an `auth` node, `POST /login` returns a signed JWT. Send `Content-Type: application/json` (a `fetch` with `JSON.stringify` and no header sends `text/plain` and gets 415). See [auth.md](auth.md).

## `kaanal.yaml`

```yaml
version: 1
seed: 42                 # same seed → same data and timing, every run
determinism: controlled  # or realistic: timings vary per run; `kaanal up` prints the seed so you can replay it
port: 8080
nodes:
  nginx: { type: gateway, flavor: nginx, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 5ms }
  db:    { type: database, flavor: postgres,
           ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:                       # optional: per-endpoint overrides (a file path also works)
  "GET /health": { override: { body: { ok: true } } }
  "GET /users":  { override: { status: 200, body: [ { id: 1 } ] } }
```

- **nodes** — eight kinds: `gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer` (see [nodes.md](nodes.md) for every field; `auth` serves login and JWKS, see [auth.md](auth.md)). The gateway is optional (at most one); without it, requests enter at the service that owns the route. A `database` needs `ops` costs for `read`/`write`/`scan`, or a `flavor` preset. `proc` is processing time. A duration is `5ms`, a percentile pair `{ p50: 4ms, p99: 20ms }` (a realistic long-tailed distribution) or `{ mean: 5ms }` (exponential, as in queueing theory). Any node may set `concurrency: N` (how many requests it serves at once). Live mode and the simulator both enforce it: extra requests queue, and the trace shows how long with `node.wait=`.
- **edges** — the dependency graph, limited to the [edge table](nodes.md#edges-who-may-call-whom). No cycles, no self or duplicate edges, no dangling references; all rejected at startup. `calls: N` sets how many times the target is called per request.
- **scenarios** — optional named chaos bundles (`scenarios:` here, or one file each in `scenarios/` next to this file), switched on live; see [scenarios.md](scenarios.md).
- **endpoints** — per-endpoint behaviour, fully described in [endpoints.md](endpoints.md). An entry with `override` serves a fixed response that replaces or extends the spec's routes. Same path + same parameter names = override; otherwise a new static endpoint, which needs a gateway.

Check a file without starting:

```bash
kaanal validate -c kaanal.yaml    # ok / errors with file:line:col, plus warnings
```

Warnings flag schema keywords the data generator ignores (like `not` or `patternProperties`) so you're never surprised by generated data that violates them.

## CLI

| Command | What it does |

| `kaanal init [SPEC] [--service NAME] [--port p] [--locale TAG] [--server-style S] [--yes] [--force]`                                             | Write a starter `kaanal.yaml` (and `.gitignore` entry) from an OpenAPI file, path or `http(s)` URL; asks for service, port, locale and server style (event-loop or thread-per-request) on a terminal, else uses defaults (`app`, 8080, `en-IN`, event-loop); writes estimated slots, queues, times and an `endpoints:` entry per route. See [init.md](init.md).                                                                                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal up [-c f] [--port p] [--host ip]`                                                                                                        | Start (foreground). Ctrl+C or `SIGTERM` stops it (drains for 5 s first).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `kaanal up -d [--wait 15s] [-c f] [--port p] [--host ip]`                                                                                        | Start in the background; returns when the server answers, with the URLs. A second `up -d` prints the running instance and exits 0. See [lifecycle.md](lifecycle.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `kaanal up --blank [-c f] [--port p] [--host ip]`                                                                                                | Start with no system and an empty draft (`-c` is where `draft save` writes it; the file is not read). The mock answers 404 until the first `kaanal draft apply`. Not with `-d`. See the [control API](control-api.md#draft).                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `kaanal studio [-c f] [--port p] [--host ip] [--no-open]`                                                                                        | `up --blank` that also opens the UI in your browser (`--no-open` only prints the URL).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `kaanal demo [--port p] [--locale TAG] [--open] \| --eject [DIR] [--force]`                                                                      | Run the demo shop with its chaos bar from `.kaanal/demo/` (foreground, `127.0.0.1` only); prints one link with the token. See [demo.md](demo.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `kaanal down [--timeout 5s]`                                                                                                                     | Stop the running instance: drain, then force-kill only a verified kaanal process. Exits 0 when nothing is running. With `--endpoint` it stops a remote instance instead: [remote down](lifecycle.md#remote-down).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `kaanal validate [-c f]`                                                                                                                         | Check config, topology and specs.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `kaanal schema [config\|dictionary\|pack]`                                                                                                       | Print the JSON Schema of `kaanal.yaml` (default), `data.yaml` or a `locales/<tag>.yaml` pack; save it and point your editor's YAML plugin at it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `kaanal endpoints [-c f] [--write]`                                                                                                              | Print skeleton entries for spec routes with none; `--write` appends them to the endpoints file.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `kaanal status`                                                                                                                                  | Table of nodes with their effective chaos (scenarios plus live) and the active scenario names.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `kaanal chaos set [node] [--route "GET /x"] [--replica N] [--latency 300ms] [--error-rate 10%] [--error 503] [--kind hang] [--down] [--for 30s]` | Change chaos live. Fields merge — set only what you name. `--for` reverts what this command sets after that long.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `kaanal node down NODE [--replica N] [--for 30s]`                                                                                                | Take a node down (503 for everything through it). With `--for` the server puts it back by itself. See [Timed chaos](#timed-chaos).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `kaanal node up NODE [--replica N]`                                                                                                              | Clear the node's live down flag; other live chaos stays. Exits 1 if a scenario or override still holds it down.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `kaanal node list`                                                                                                                               | The node table `status` prints (name, type, flavor, `up`/`down`, chaos), with `(reverts in 27s)` on timed chaos.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `kaanal chaos clear [node] [--route "GET /x"] [--replica N]`                                                                                     | Clear one node, one endpoint or one node-at-endpoint entry, or all live chaos with no argument (scenarios stay).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `kaanal draft show\|set -f FILE\|validate [-f FILE]\|apply [--timeout T]\|save [--confirm-loss] [--fix-scenarios remove]`                        | Change the running system without a restart: `show` prints the draft (the `kaanal.yaml` text the instance started from), `set -f FILE` replaces it, `validate` checks it (or a file) and prints a problem as `file:line:col: message` (exit 1), `apply` makes it the running system, `save` writes it to `kaanal.yaml` (atomically; it refuses if the file changed on disk since the instance read it, and with `--fix-scenarios remove` it also deletes scenario files the draft no longer resolves against). Chaos, active scenarios and pins are kept where the node, edge, replica or endpoint still exists; the rest is listed. Route counters restart. See [control API](control-api.md#draft). |
| `kaanal sim run -p NAME                                                                                                                          | -f FILE [\|-f FILE [--duration D] [--warmup D] [--seed N] [--scenario S] [--no-wait]`,`sim list`,`sim show ID`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `kaanal sample "GET /users/{id}" [--seed N]`                                                                                                     | Print one generated response body of an endpoint (its size and status go to stderr).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `kaanal skeleton [-o FILE] [--force]`                                                                                                            | Print, or write, a generated OpenAPI file for the draft's `endpoints:` (use it as a node's `spec:` when you have no spec).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `kaanal scenario list\|show NAME\|activate NAME\|deactivate NAME\|set A B\|clear\|active\|load FILE\|save NAME`                                  | Named chaos scenarios: switch on and off, stack, load a file, save what you built. See [scenarios.md](scenarios.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `kaanal cycle EP [--state S] [--status N] [--delay D] [--off]`                                                                                   | Pin an endpoint to the next UI state (loading, empty, error, slow, success), or to one with `--state`; `--off` removes it. See [pins.md](pins.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `kaanal pins`                                                                                                                                    | Live pins and the pin in force per endpoint, with its source.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `kaanal logs [-n N] [-f] [--since D] [--status S] [--failed] [--route "GET /x/{id}"] [--scope NAME]`                                             | The last finished requests, one line each; `-f` follows live. See [observability.md](observability.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `kaanal trace ID\|last\|CLIENT_ID`                                                                                                               | One request as a per-hop waterfall: Kaanal's id (from `X-Request-Id`), `last`, or the id your client sent. See [observability.md](observability.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `kaanal completions bash\|zsh\|fish\|powershell\|elvish`                                                                                         | Print a shell completion script to stdout; needs no instance. See [Shell completions](#shell-completions).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `kaanal reset`                                                                                                                                   | Clean slate for tests: scenarios off, live chaos cleared, counters and rate limits reset, and the request log emptied (ids carry on).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

### Global flags

They may go before or after the subcommand. They apply to every command that talks to an instance (`status`, `chaos`, `scenario`, `cycle`, `pins`, `logs`, `trace`, `reset`, `down`).

| Flag             | What it does                                                                                                                                        |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--json`         | Print the server's JSON instead of text (also `KAANAL_JSON=1`). Errors stay text on stderr. It changes only what is printed.                        |
| `--endpoint URL` | Talk to the instance at `http://HOST:PORT` instead of the one in this folder (also `KAANAL_ENDPOINT`): `kaanal --endpoint http://host:8080 status`. |
| `--token T`      | The control token for `--endpoint` (also `KAANAL_TOKEN`, which is the better way: see [Scripting and CI](#scripting-and-ci)). Needs an endpoint.    |

On `logs`, `chaos set` and `chaos clear` the route-key flag is `--route "GET /x"`; the old spelling `--endpoint "GET /x"` still works there for one release (a value with a space is read as a route key, a URL as the instance). A flag beats its environment variable; an empty variable counts as unset. With an endpoint, Kaanal never looks at `.kaanal/` in this folder, and `up` / `up -d` refuse it (exit 64). The control API is plain http, so `https://` is refused; a non-loopback host prints `warning: sending the token over plain http to HOST` on stderr. If you reach the instance through a port mapping, map it to the same port number the instance binds (Kaanal sends `Host: localhost:PORT`).

### Exit codes

| Code | Meaning                                                                                                                                                           |
| ---- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0`  | Ok (also `down` with nothing running, and `up -d` on a running instance)                                                                                          |
| `1`  | Error: bad config, startup failure, a value the server refused (HTTP 400), an instance that does not answer; with `--endpoint`, cannot reach it or token rejected |
| `2`  | No running local instance (or only stale state, which is cleaned up); never with `--endpoint`                                                                     |
| `3`  | Reserved for assertion failures; no command returns it yet                                                                                                        |
| `64` | Usage error: a wrong command line, found before anything is sent (unknown flag, bad `--ttl`, `-n 0`, bad `--endpoint`, no token, ...)                             |

`--help` and `--version` exit 0. The details, and what each case prints, are in [lifecycle.md](lifecycle.md#exit-codes).

### Scripting and CI

```bash
kaanal status --json | jq '.[] | select(.type == "database")'   # one JSON line on stdout
export KAANAL_ENDPOINT=http://10.0.0.5:8080                      # an instance elsewhere
export KAANAL_TOKEN=...                                          # its token (from its run.json)
kaanal chaos set db --down && kaanal --json pins
```

A CI step that breaks the database for a while and then checks the app, with nothing to clean up if the job dies:

```bash
kaanal up -d
kaanal node down db --for 2m           # reverts on the server, even if this job is killed
npx playwright test checkout-degraded
kaanal node up db                      # optional: end it early (exit 1 if a scenario holds db down)
kaanal down
```

- Read commands print the server's JSON as one line; write commands print the server's answer when there is one and nothing when the server sends none. `logs` prints one JSON object per line.
- `--json` never changes what a command does or its exit code. Errors stay text on stderr, so a script reads `$?`, not an error object.
- Set the token in `KAANAL_TOKEN`. A value given with `--token` is visible in the process list and your shell history. Kaanal never prints the token. An exported `KAANAL_TOKEN` alone does nothing to local commands: only `--endpoint` or `KAANAL_ENDPOINT` switches to a remote instance (and only the `--token` flag without an endpoint is a usage error).
- A remote instance that cannot be reached is exit 1, not 2, so `$? -eq 2` still means "nothing is running in this folder".

### Shell completions

`kaanal completions SHELL` prints a script for `bash`, `zsh`, `fish`, `powershell` or `elvish`. Completion is static: subcommands, flags and fixed values (it does not look up node or scenario names), and it also lists hidden flags such as `--detached`.

```bash
# bash
kaanal completions bash > ~/.local/share/bash-completion/completions/kaanal

# zsh (the folder must be in $fpath, before compinit)
kaanal completions zsh > ~/.zfunc/_kaanal

# fish
kaanal completions fish > ~/.config/fish/completions/kaanal.fish

# PowerShell: add to your $PROFILE
kaanal completions powershell | Out-String | Invoke-Expression

# elvish: add to ~/.config/elvish/rc.elv
eval (kaanal completions elvish | slurp)
```

Open a new shell afterwards.

## What a request looks like

```bash
curl -i http://localhost:8080/users/42
```

```http
HTTP/1.1 200 OK
x-kaanal-trace: nginx.proc=1.0ms,net=1.0ms,app.proc=5.0ms,net=1.0ms,db.read=4.2ms
x-request-id: r04f2-0042
access-control-allow-origin: *

{ "id": 42, "firstName": "Priya", "email": "priya.iyer42@example.com", ... }
```

- The **body** is generated from your OpenAPI response schema — real-looking names, emails, pincodes — and is identical on every call to the same URL. Path params flow into matching fields (`id: 42`).
- The **timing** is what the trace header adds up to: hops in sequence add; a `parallel` block costs its slowest branch. The client actually waits it.
- A `parallel` step shows its branches in brackets, split by `|`, and a cache lookup shows `hit` or `miss`: `nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms`. A failure ends the header with `node.error=STATUS`. Very long chains end with `,...+N hops`, where N is the number of hops left out.
- When a node is full, a request queues and the trace shows it before the hop it waited for: `...,net=1.0ms,db.wait=98.0ms,db.read=100.0ms`. A node with a `rate_limit` answers `429` with a `Retry-After` header (whole seconds) once its tokens run out.
- `X-Request-Id` is the request's id. Run `kaanal trace r04f2-0042` to see that request hop by hop, or `kaanal logs` for the latest ones; the 1000 most recent are kept. See [observability.md](observability.md).
- **CORS** is wide open on mock routes, so any local frontend just works.

## Data and locales

Generated bodies follow your OpenAPI response schemas. Three things shape what the values look like: the **locale** (whose names, places, phones and currency), the **dictionary** (which generator fills which field name) and the schema keywords themselves. Same Kaanal version, config, data files and `seed:` give the same bytes every time.

### Locale

```yaml
version: 1
locale: en-IN              # the project default; en-IN if omitted
nodes:
  app:    { type: service, spec: openapi.yaml }
  us-api: { type: service, spec: us.yaml, locale: en-US }   # this node's routes use en-US
```

- `locale:` is a tag such as `en-IN` or `en-US`. A node's own `locale:` beats the project default; the locale comes from the node that owns the route.
- Seven packs are built in: `en-IN` (names, `+91`, `INR`, `India`), `en-US` (`+1`, `USD`, `United States`), `en-GB` (`+44`, `GBP`), `de-DE` (`+49`, `EUR`, no states), `fr-FR` (`+33`, `EUR`), `es-ES` (`+34`, `EUR`) and `pt-BR` (`+55`, `BRL`, states).
- Any other tag needs a pack: drop `locales/<tag>.yaml` next to `kaanal.yaml`. A pack with a built-in's tag replaces it. [Data packs](data-packs.md) shows how to write one and where Kaanal looks.
- An unknown locale is a startup error that lists the available tags (and says `did you mean` when only the case differs). `kaanal validate` and `kaanal up` check every pack in `locales/`, used or not.

### Dictionary

`data.yaml` next to `kaanal.yaml` (or `data: path/to/file.yaml` at the top level) overrides the built-in field dictionary of about 150 names:

```yaml
version: 1
fields:
  price:  { gen: money, min: 99, max: 4999 }
  status: { gen: pick, from: [paid, refunded, pending] }
  sku:    { gen: template, value: "SKU-{digits:6}" }
  "*id":  [{ gen: uuid }, { gen: integer, min: 1, max: 99999 }]
```

Field names match whatever their style (`userEmail`, `user_email`); a `*suffix` key matches whole trailing words only (`*id` matches `userId`, not `paid`); an entry can be a list, the first generator that fits the field's type applying. All of it, the generator list and the error messages are in [Data packs](data-packs.md#writing-a-dictionary).

Precedence, first match wins:

1. schema `example`, then `examples` (a response-level `example`/`examples` beats the schema's own);
2. `const`, then `enum`;
3. `oneOf`/`anyOf`: the seed picks a branch;
4. your `data.yaml`;
5. `format`;
6. the built-in dictionary;
7. type and bounds defaults.

A string with a `pattern` matches it: a generated value that already matches (an email under `.+@.+`) is kept, otherwise Kaanal samples one from the regex (ASCII only, open repeats up to 8 extra, `minLength`/`maxLength` respected). Your `example`, `enum`, `const` and `data.yaml` `static`/`pick` values are never replaced. A pattern Kaanal cannot sample (lookaround, back-references, a huge repeat) or cannot fit within the length bounds is a startup warning, and the value may not match it.

The schema's type and bounds always win over a dictionary entry: numbers are clamped to `minimum`/`maximum`, generated strings fitted to `maxLength` and `minLength` (in characters). An entry that cannot fit its field is skipped. A misfit in your `data.yaml` is a warning from `kaanal validate` and `kaanal up` that names the endpoint and field; a misfit built-in entry is skipped silently. Your own `pick`, `static` and `template` strings are never trimmed or padded: they are sent as written, and one that can fall outside the field's `minLength`/`maxLength` is a warning naming the field.

### Schema keywords

| Keyword                           | What Kaanal does                                                                                                                                                                                                      |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `example`, `examples`             | Used verbatim; `examples` (a list, or an OpenAPI map of `{ value }`) is picked from by seed.                                                                                                                          |
| `const`                           | The constant.                                                                                                                                                                                                         |
| `format`                          | Adds `hostname`, `ipv4`/`ipv6` (documentation ranges only: `198.51.100.x`, `203.0.113.x`, `2001:db8::`), `byte`, `password`, `duration` and `int32`/`int64` bounds to `email`, `uuid`, `date-time`, `date` and `uri`. |
| `oneOf`, `anyOf`                  | The seed picks one branch, uniformly. Adding or reordering other properties never changes the pick.                                                                                                                   |
| `nullable`, `type: [x, "null"]`   | Read; a real value is always generated, never `null`.                                                                                                                                                                 |
| `additionalProperties` (a schema) | 0 to 3 extra seeded keys (`key1234`); none for `false`, `true` or an absent keyword.                                                                                                                                  |
| `discriminator`                   | The chosen branch gets its tag value; see below.                                                                                                                                                                      |

#### Discriminators

With `oneOf`/`anyOf` and a `discriminator`, the picked branch's tag property is set so the body matches exactly one branch:

- With a `mapping`, the tag is a mapping key that names that branch (by `#/components/schemas/Dog` or just `Dog`). A branch named by several keys gets one of them, by seed.
- Without a `mapping`, the tag is a single-value `enum` or a `const` on the discriminator property of the branch. Any non-empty `enum` counts.
- There is **no implicit tag** from the component name: with neither a mapping nor an `enum`/`const`, the tag is generated like any other field.
- An object-level `example` on a branch beats the tag.

`kaanal validate` and `kaanal up` warn about the cases that cannot work:

- `discriminator 'petType' has no mapping, enum or const for oneOf branch 1; its tag is generated normally`
- `discriminator mapping 'cat' names no oneOf/anyOf branch; it is never generated`
- `discriminator has no propertyName and is ignored`

It also samples each endpoint's generated response (5 bodies) and checks them against the response schema. A miss is a warning that names the endpoint and the path, for example `GET /orders: 3 of 5 generated samples violate the response schema: /items/*/total: maximum`. It is Kaanal's bug or a contradiction in the spec (an `example` outside its own `maximum`); the mock still starts. `kaanal up` prints these lines a moment after it has started serving, so a big spec never slows startup.

### Behaviour changes in this release

Kaanal is pre-1.0, and the same version, config, files and seed give the same data, but data may change between `v0.0.x` releases. If you diff generated bodies or commit them as fixtures, expect these differences:

- All built-in output is new: `en-IN` names, places, phones and addresses come from a larger pack, and other values may draw differently. Fixtures that must never change belong in an `override:` body or a schema `example`.
- `oneOf` and `anyOf` used to always generate the first branch; they now pick a branch by seed.
- `ipv4` values moved to documentation ranges, `ipv6` and `hostname` are new (`2001:db8::...`, `*.example.com`).
- `street` is a street name (it used to be the full address), `status` picks from `active`, `inactive` and `pending` (it was always `active`), and any field ending in `At` or `_at` (`shippedAt`, `expires_at`) is a timestamp.
- Suffix matching is by whole word. An all-lower-case compound such as `unitprice` no longer matches the money suffixes (`unitPrice` and `unit_price` do), and `valid`, `paid` or `android` never match `*id`.
- New keywords change how much of a body's value budget a schema uses (`const`, `additionalProperties`, `oneOf`), so a very large nested schema may reach its cap at a different place.
- `discriminator` is no longer reported as an ignored keyword.
- A field ending in `Total`/`_total` (`orderTotal`, `grandTotal`) or `subtotal` is money now, number and integer alike. A bare `total` stays a count from 0 to 1000, and is a whole number on a `number` field too.
- `street` and the street inside `address` are now the same street for one object.
- Records now agree with themselves (D2): `updatedAt` is never before `createdAt` (and the other timestamp chains), `age` matches `dateOfBirth`, an order's `lineTotal`, `subtotal`, `tax`, `shipping`, `discount` and `total` add up, and a string with a `pattern` matches it. `multipleOf` and `uniqueItems` (scalar items) are honoured. A field you pinned with `example`, `enum`, `const` or a `static`/`pick` entry is never rewritten.
- Values are spread more like production (D2): prices lean cheap and end in `.99`, `.00`, `.95`, status words favour `paid`/`active`/`delivered`, flags such as `active` are mostly true, `rating` leans positive, `quantity` is mostly 1 to 3, timestamps lean recent, `age` has more people between 33 and 64. Every value stays possible. Fixtures that must never change belong in an `override:` body or an `example`.
- `kaanal validate` and `kaanal up` now also generate five bodies per endpoint and check them against the response schema; see Warnings above.
- Built-in packs carry coordinate bounds (`geo:`): `latitude`/`longitude` stay inside India for `en-IN` (unchanged) and inside the continental US for `en-US` (they were India's for every locale); a pack without `geo:` uses the whole globe.
- `empty` pin bodies use the owning node's locale and your `data.yaml` (they always used `en-IN` and the built-ins), and each field draws from its own seed, so the values change.
- `en-IN` phone numbers are `+91 9` and nine digits, which can look like a real Indian mobile number. Use them as test data only; never send messages to them.

## Chaos, live

```bash
kaanal chaos set db --latency 300ms        # every db hop +300 ms
kaanal chaos set db --error-rate 10% --error 504
kaanal chaos set app --down                # hard outage: 503 for everything through app
kaanal status                              # see what's active
kaanal chaos clear db                      # back to normal
```

A failure you set can go unseen: a warm cache answers before the database is reached, and a node no route calls is never visited. If the run shows no failures although a node is down, read the trace (it names each hop), or break the cache too: `kaanal chaos set cache --down` makes every read reach the database. Studio shows this as a note under the chaos buttons and on Results. See [concepts](concepts.md#chaos).

### Timed chaos

Add `--for` and the server undoes it for you, even if your script or terminal is gone:

```bash
kaanal node down db --for 30s              # db: down (reverts in 30s)
kaanal node list                           # db shows "down ... (reverts in 27s)"
kaanal chaos set db --latency 500ms --for 5m
kaanal node up db                          # bring it back early
```

- `--for` takes 100 ms to 24 h (`500ms`, `30s`, `5m`, `2h`); anything else is a usage error (exit 64) before anything is sent. It exists on `chaos set` and `node down` only.
- It leases what the command sets, field group by field group: latency, errors (rate, kind and status together), down and mode. `kaanal chaos set db --latency 200ms` (permanent) followed by `kaanal node down db --for 30s` expires only the outage; the latency stays.
- A command without `--for` makes the fields it sets permanent again: a plain `kaanal node down db` after a timed one keeps db down until you bring it up. Setting a field again with a new `--for` restarts its clock.
- When time is up the fields are **cleared**, not restored to an earlier value. `clear`, `reset` and `node up` end the lease too. Leases live in memory and are gone after a restart.
- `node up` only removes the live down flag. If an active scenario holds the node down, it prints `db is still down: held by an active scenario or override (see kaanal scenario active)` and exits 1, so `kaanal node up db && run-tests` does not run against a database that is still down.
- A timed chaos is not a [timed scenario](scenarios.md#timed-chaos-and-timed-scenarios): `--for` leases hand-set chaos, `scenario activate --ttl` leases an activation.

For a whole situation at once, define a [scenario](scenarios.md) and switch it on:

```bash
kaanal scenario activate payments-down     # defined under scenarios: in kaanal.yaml
kaanal scenario activate payments-down --ttl 30s   # turns itself off after 30 s
kaanal scenario deactivate payments-down
kaanal reset                               # start a test from a clean slate
```

Failures propagate upstream the way real ones do: a failing database makes the request fail with the database's status, after spending the time of the hops before it. The trace header names the failing node (`...,db.error=504`).

## The status page

Open the UI URL printed by `kaanal up`. It shows a node table with live rps/error% and chaos controls (latency, error %, status, down), plus a live request log. Details: [web-ui.md](web-ui.md).

## Safety notes

- Kaanal binds `127.0.0.1`. `--host 0.0.0.0` works but warns; the control API still requires the token and a loopback `Host` header.
- The token is per-run and lives in `.kaanal/run.json`, readable only by you. The background process's log (`.kaanal/kaanal.log`) never contains it.
- `KAANAL_CONFIG_DIR` sets the base folder for the auth signing key (default: your per-user local config folder). See [auth.md](auth.md).
- All generated data is synthetic. Emails always use `example.com`.
