---
title: "Start a project: kaanal init"
sidebar:
  order: 2
---

`kaanal init` turns an OpenAPI 3.0 file into a project you can start straight away.

```
kaanal init openapi.yaml
kaanal up
```

It writes `kaanal.yaml` in the current folder: a gateway, one service that serves your spec, a database and a cache, with starting numbers for slots, queues and times, and an `endpoints:` block with one entry per route of your spec, so a request takes a realistic time instead of about 0 ms. **Every number is an estimate** for a small deployment (the same starting numbers as Studio's new nodes), labelled so in comments in the file. Edit freely; change `kaanal.yaml` and the Studio shows what you chose.

| Node            | What `init` writes                                                                                                                                                                                                           |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| gateway         | `concurrency: { limit: 256, queue: 100 }`, `proc: 2ms`, `timeout: 30s` (clients and proxies usually give up after 30 s to 60 s; a request through the gateway that takes longer fails as timed out)                          |
| your service    | `concurrency: { limit: 32, queue: 100 }` and `proc: 25ms` for an event loop; `hold: call` and `concurrency: { limit: 200, queue: 100 }` for a thread per request (see [nodes](/docs/reference/nodes/), "What a slot covers") |
| database, cache | the `postgres` and `redis` flavor presets, unchanged                                                                                                                                                                         |
| every edge      | `net: { p50: 1ms, p99: 3ms }`                                                                                                                                                                                                |

No payload sizes and no `per_kb` are written.

### The `endpoints:` block

One entry per route of the spec, chosen by method, running under the service that owns the route (see [endpoints](/docs/guides/mock/endpoints/)):

| Route                                                                                                  | Entry                                                                          |
| ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| `GET` on a collection (the path does not end in a `{param}`), such as `/users` or `/orders/{id}/items` | `op: scan`, then `call: database`                                              |
| `GET` by id (the path ends in a `{param}`; a trailing `/` is ignored), such as `/users/{id}`           | `op: read`, then `cache: cache`, then `call: database`                         |
| `POST`, `PUT`, `PATCH`, `DELETE`                                                                       | `op: write`, then `call: database` (no cache)                                  |
| any other method (`HEAD`, `OPTIONS`, `TRACE`)                                                          | no entry: the default chain applies (a `HEAD` with no entry follows its `GET`) |

The keys are exactly the route keys `kaanal endpoints` prints, including a server base path. If every route is one of the last kind, no `endpoints:` block is written. `kaanal endpoints` lists the routes that have no entry (a `HEAD` or `OPTIONS` your spec declares), for you to refine later.

### The `traffic:` block

After `endpoints:`, `init` writes one profile named `peak` (the name Studio uses) so `kaanal sim run -p peak` works at once: `rps: 100`, `duration: 30s`, and one journey for each entry of `endpoints:` (same keys), weight 3 for a `GET` and 1 for `POST`, `PUT`, `PATCH` and `DELETE`. A comment above it says the numbers are estimates. If the spec has no route that gets an entry, no `traffic:` section is written, and a spec with more than 64 such routes gets journeys for the first 64. See [traffic](/docs/guides/simulate/traffic/).

It also adds `.kaanal/` to `.gitignore` (creating the file if needed, never twice).

## Questions

On a terminal, `init` asks up to four questions, each with its default in brackets. Press Enter to accept:

| Question     | Default                  | Flag             |
| ------------ | ------------------------ | ---------------- |
| Service name | `app`                    | `--service`      |
| Port         | `8080`                   | `--port`         |
| Locale       | `en-IN` (generated data) | `--locale`       |
| Server style | `event-loop`             | `--server-style` |

The fourth question reads `Server style: event-loop (Node, Go, async Python) or thread-per-request (Tomcat, sync Gunicorn, Puma)? [event-loop]`. Answer `event-loop` or `thread-per-request`, exactly. An event loop holds a slot only while the service does its own work; a thread per request holds it until everything the service called has answered, so a slow database fills its slots too.

A flag answers its question, so it is not asked. With no spec argument on a terminal, `init` also asks for the spec path. The questions go to stderr, so `--json` output stays clean.

A bad typed answer is explained and asked again (three tries, then exit 64). A bad flag value is refused at once with exit 64. At the end of input the default is used.

`--yes` (or any run where stdin is not a terminal, such as CI) takes every default and never waits. With no spec argument and no terminal, `init` stops with exit code 64.

The service name is one lowercase letter, then lowercase letters, digits, `-` or `_`, up to 32 characters, and cannot be `gateway`, `database` or `cache`.

## Safety

- An existing `kaanal.yaml` is never overwritten unless you pass `--force`.
- The file is checked the same way as `kaanal validate` before it appears. If the spec is missing, is not OpenAPI 3.0, or has no paths, `init` exits 1 and leaves your folder unchanged. Errors name `kaanal.yaml`.
- A leftover `kaanal.yaml.tmp` (an interrupted run) is never overwritten: delete it and retry.
- `.gitignore` is only appended to, in its own line ending. If it cannot be read as UTF-8 or cannot be written, `init` leaves it alone, prints a warning on stderr and still succeeds; add `.kaanal/` yourself. Lines such as `.kaanal`, `/.kaanal/` or `.kaanal/*` count as already ignored.

## From a URL

```bash
kaanal init https://example.com/api/openapi.yaml
```

`init` downloads the spec next to `kaanal.yaml`, named after the last URL path segment when ending in `.yaml`, `.yml`, or `.json` (otherwise `openapi.yaml`; invalid chars become `_`, and Windows device names or >100-char names fall back to `openapi.yaml`).

- **Existing files**: stops with exit `64` unless `--force` is passed (`--force` replaces it and prints `Replaced the existing openapi.yaml`; with `--json`, a `replaced` list; the old file is kept as `<name>.kaanal-old` during download; refuses if that backup exists).
- **Download rules**: single `GET` (no cookies/credentials; a URL with a user name or password such as `https://user:pass@...` exits `64`); `https` for any host, plain `http` only for `localhost`/`127.0.0.0/8`/`::1`; max `3` redirects (never `https` → `http`); `10s` / `10 MB` cap; must be OpenAPI 3.0 (else exit `1`).
- **Proxies & TLS**: `https` honours `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; `http` skips proxies. Failures clean up downloaded/backup files without printing response bodies or query strings. (`https` requires the default `https` Cargo feature: rustls with the `ring` crypto provider and the Mozilla root certificates from `webpki-roots`, and a C compiler to build; a build with `--no-default-features` has no TLS, refuses an `https://` URL with exit `64`, and needs the file downloaded first and its path passed instead.)

## Output

```
Wrote kaanal.yaml, .gitignore
Chose: event loop, 32 slots, queue 100, 30 s timeout (estimates; edit kaanal.yaml)
Next: kaanal up
```

The `Chose:` line says what the file holds (`thread per request, 200 slots, queue 100, 30 s timeout` for the other style); the numbers are estimates, not measurements. With `--json` the `Chose:` line is not printed and the output keeps its shape: `{"written":["kaanal.yaml",".gitignore"]}`. `.gitignore` is listed only when it changed; a spec downloaded from a URL is listed after `kaanal.yaml`, and one that replaced an existing file is also in `"replaced"`.

`init` writes local files, so it refuses to run when `--endpoint` or `KAANAL_ENDPOINT` is set.
