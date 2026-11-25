---
title: "CLI"
sidebar:
  order: 2
head:
  - tag: style
    content: ".sl-markdown-content tr > :is(th, td):not(:last-child) code { white-space: nowrap; }"
---

All commands that talk to a running instance read `.kaanal/run.json` in the current directory unless `--endpoint` is set.

## Lifecycle & authoring

| Command                           | Options                                                                              | What it does                                                                                                                                                                                                                                                                                                                              |
| --------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal init [SPEC]`              | `--service NAME`, `--port P`, `--locale TAG`, `--server-style S`, `--yes`, `--force` | Generates `kaanal.yaml` (and a `.gitignore` entry) from an OpenAPI file or `http(s)` URL. Prompts for service, port, locale and server style (`event-loop` or `thread-per-request`; defaults `app`, `8080`, `en-IN`, `event-loop`), writing estimated slots, queues, times and `endpoints:` entries. See [init](/docs/guides/mock/init/). |
| `kaanal up`                       | `-c FILE`, `--port P`, `--host IP`                                                   | Starts the mock server in the foreground (`Ctrl+C` or `SIGTERM` drains for up to `5s`).                                                                                                                                                                                                                                                   |
| `kaanal up -d`                    | `--wait 15s`                                                                         | Daemonizes and returns once healthy, with the URLs (idempotent: exit `0` if already running). See [Lifecycle](/docs/guides/observe/lifecycle/).                                                                                                                                                                                           |
| `kaanal up --blank`               | (cannot combine with `-d`)                                                           | Starts empty with no system; mocks return `404` until `draft apply`. See [Draft API](/docs/reference/control-api/draft/).                                                                                                                                                                                                                 |
| `kaanal studio`                   | `-c FILE`, `--port P`, `--host IP`, `--no-open`                                      | Runs `up --blank` and opens Studio in your browser (`--no-open` prints the URL only).                                                                                                                                                                                                                                                     |
| `kaanal demo`                     | `--port P`, `--locale TAG`, `--open`; or `--eject [DIR] [--force]`                   | Runs the built-in demo shop with a chaos bar from `.kaanal/demo/` (`127.0.0.1` only), or ejects editable files with `--eject`. See [the demo](/docs/start/tour/).                                                                                                                                                                         |
| `kaanal down`                     | `--timeout 5s`                                                                       | Stops the running instance: drains, then force-kills only a verified `kaanal` PID; exits `0` when nothing is running. With `--endpoint` it stops a remote instance: [remote down](/docs/guides/observe/lifecycle/#remote-down).                                                                                                           |
| `kaanal validate`                 | `-c FILE`                                                                            | Validates config, topology and OpenAPI specs without starting.                                                                                                                                                                                                                                                                            |
| `kaanal schema [KIND]`            | `config` (default), `dictionary` or `pack`                                           | Prints the JSON Schema for `kaanal.yaml` (`config`), `data.yaml` (`dictionary`, the field dictionary) or `locales/<tag>.yaml` (`pack`, a locale pack).                                                                                                                                                                                    |
| `kaanal endpoints`                | `-c FILE`, `--write`                                                                 | Prints skeleton `endpoints:` entries for uncovered spec routes (`--write` appends them to the endpoints file).                                                                                                                                                                                                                            |
| `kaanal skeleton`                 | `-o FILE`, `--force`                                                                 | Prints or writes a generated OpenAPI 3.0 file from the draft's `endpoints:`.                                                                                                                                                                                                                                                              |
| `kaanal sample "GET /users/{id}"` | `--seed N`                                                                           | Prints one generated response body (status and byte size go to stderr).                                                                                                                                                                                                                                                                   |

## Chaos, pins & draft

| Command                           | Options                                                                                                                                 | What it does                                                                                                                                                                                                                       |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal status`                   |                                                                                                                                         | Node table with effective chaos (scenarios + live) and the active scenario names.                                                                                                                                                  |
| `kaanal chaos set [node]`         | `--route "GET /x"`, `--replica N`, `--latency 300ms`, `--error-rate 10%`, `--error 503`, `--kind hang`, `--down`, `--for 30s`           | Merges live chaos fields (`--for` auto-reverts).                                                                                                                                                                                   |
| `kaanal chaos set --edge FROM:TO` | `--latency 20ms`, `--jitter 5ms`, `--error-rate 10%`, `--kind reset` or `--kind hang`, `--down`, `--bandwidth-cap 500KB/s`, `--for 30s` | Chaos on a link instead of a node (`--edge app:db`). It cannot be combined with a node, `--route` or `--replica`. `--bandwidth-cap` takes bytes per second or a unit (`500KB/s`) and only works on an edge that has a `bandwidth`. |
| `kaanal chaos clear [node]`       | `--route "GET /x"`, `--replica N`, `--edge FROM:TO`                                                                                     | Clears live chaos on a target, a link or all nodes (scenarios stay).                                                                                                                                                               |
| `kaanal node down NODE`           | `--replica N`, `--for 30s`                                                                                                              | Marks a node `503` ([Timed chaos](/docs/guides/break/chaos-live/#timed-chaos)).                                                                                                                                                    |
| `kaanal node up NODE`             | `--replica N`                                                                                                                           | Clears live `down` (exits `1` if a scenario or override still holds it down).                                                                                                                                                      |
| `kaanal node list`                |                                                                                                                                         | Prints the node status table with the remaining timed-chaos countdowns.                                                                                                                                                            |
| `kaanal scenario`                 | `list`, `show NAME`, `activate NAME`, `deactivate NAME`, `set A B`, `clear`, `active`, `load FILE`, `save NAME`                         | Manages named chaos scenarios. See [Scenarios](/docs/guides/break/scenarios/).                                                                                                                                                     |
| `kaanal cycle EP`                 | `--state S`, `--status N`, `--delay D`, `--off`                                                                                         | Steps or pins an endpoint (`loading`, `empty`, `error`, `slow`, `success`). See [Pins](/docs/guides/break/pins/).                                                                                                                  |
| `kaanal pins`                     |                                                                                                                                         | Lists the active pins.                                                                                                                                                                                                             |
| `kaanal reset`                    |                                                                                                                                         | Clears scenarios, live chaos, counters, rate-limit buckets and the request log (request IDs continue).                                                                                                                             |
| `kaanal draft`                    | `show`, `set -f FILE`, `validate [-f FILE]`, `apply [--timeout T]`, `save` with optional `--confirm-loss` and `--fix-scenarios remove`  | Inspects, edits, validates (`file:line:col: message`), applies live without a restart, or saves `kaanal.yaml`. See [Draft API](/docs/reference/control-api/draft/).                                                                |

## Simulation & observability

| Command                    | Options                                                                                                                                                                                                                                                                                                                                          | What it does                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| `kaanal sim run`           | `-p NAME` or `-f FILE`, `--duration D`, `--warmup D`, `--seed N`, `--scenario S`, `--no-wait`                                                                                                                                                                                                                                                    | Runs a seeded virtual-time simulation.                                                                |
| `kaanal sim sweep`         | `-p NAME` or `-f FILE`, `--p99 1s`, `--errors 1%`, `--max-factor 8`, `--seed N`, `--scenario S`, `--no-wait` or `--report FILE [--force]`                                                                                                                                                                                                        | Steps the load up until p99 or errors cross the limits and reports the knee.                          |
| `kaanal sim plan`          | `-p NAME` or `-f FILE`; `--target 3x` or `--target 500`; `--p99 1s`, `--errors 1%`, `--budget 2.00`, `--keep NODE` (repeatable), `--max-steps N`, `--seed N`, `--scenario S`; `--cheapest` with `--time 150s`, `--run-cap 200`, `--providers gcp,azure` (or `--providers none`); `--no-wait` or `--report FILE` (with `-o plan.yaml`); `--force` | Walks (or, with `--cheapest`, searches) for a design that holds the target load.                      |
| `kaanal sim list`          |                                                                                                                                                                                                                                                                                                                                                  | Lists runs.                                                                                           |
| `kaanal sim show ID`       |                                                                                                                                                                                                                                                                                                                                                  | Inspects one run record.                                                                              |
| `kaanal sim report ID`     | `--json`, `-o FILE [--force]`                                                                                                                                                                                                                                                                                                                    | Outputs the Markdown or JSON report. See [Simulation](/docs/guides/simulate/simulation/).             |
| `kaanal logs`              | `-n N`, `-f`, `--since D`, `--status S`, `--failed`, `--route "GET /x/{id}"`, `--scope NAME`                                                                                                                                                                                                                                                     | Prints recent requests (`-f` follows live). See [Observability](/docs/guides/observe/observability/). |
| `kaanal trace TARGET`      | `TARGET` is a request `ID`, `last` or a `CLIENT_ID`                                                                                                                                                                                                                                                                                              | Prints a per-hop waterfall for a request. See [Observability](/docs/guides/observe/observability/).   |
| `kaanal completions SHELL` | `bash`, `zsh`, `fish`, `powershell`, `elvish`                                                                                                                                                                                                                                                                                                    | Prints a static shell completion script. See [Shell completions](#shell-completions).                 |

## Global flags

Accepted before or after subcommands for instance-talking commands (`status`, `chaos`, `scenario`, `cycle`, `pins`, `logs`, `trace`, `reset`, `down`):

| Flag             | What it does                                                                                                                           |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `--json`         | Print server JSON instead of text (also `KAANAL_JSON=1`). Errors stay text on stderr.                                                  |
| `--endpoint URL` | Target `http://HOST:PORT` instead of local `.kaanal/` (also `KAANAL_ENDPOINT`). Refuses `https://` and `--endpoint` with `up`/`up -d`. |
| `--token T`      | Bearer token for `--endpoint` (prefer `KAANAL_TOKEN` env var so tokens stay out of process lists/history).                             |

On `logs`, `chaos set`, and `chaos clear`, use `--route "GET /x"` (legacy `--endpoint "GET /x"` still works for one release). Flags beat env vars; empty env vars are unset. Non-loopback hosts warn `warning: sending the token over plain http to HOST` on stderr.

## Exit codes

| Code | Meaning                                                                                   |
| ---- | ----------------------------------------------------------------------------------------- |
| `0`  | Ok (also `down` with nothing running, and `up -d` on a running instance)                  |
| `1`  | Error: bad config, startup failure, server `400`, unreachable instance, or rejected token |
| `2`  | No running local instance (or cleaned stale state); never returned with `--endpoint`      |
| `3`  | Reserved for assertion failures                                                           |
| `64` | Usage error: invalid flags/arguments caught locally before any request                    |

`--help` and `--version` exit `0`. See [Lifecycle](/docs/guides/observe/lifecycle/#exit-codes).

## Scripting and CI

```bash
kaanal status --json | jq '.[] | select(.type == "database")'
export KAANAL_ENDPOINT=http://10.0.0.5:8080
export KAANAL_TOKEN=...
kaanal chaos set db --down && kaanal --json pins
```

```bash
kaanal up -d
kaanal node down db --for 2m           # auto-reverts on the server even if the job dies
npx playwright test checkout-degraded
kaanal node up db                      # optional: clear early (exits 1 if a scenario holds it down)
kaanal down
```

- Read commands print one compact JSON line; `logs` prints one JSON object per line.
- `--json` never alters exit codes, and `KAANAL_TOKEN` alone does nothing without `--endpoint` / `KAANAL_ENDPOINT`.
- Unreachable `--endpoint` targets exit `1` (not `2`) so `$? -eq 2` keeps its local meaning.

## Shell completions

```bash
# bash
kaanal completions bash > ~/.local/share/bash-completion/completions/kaanal

# zsh (folder must be in $fpath before compinit)
kaanal completions zsh > ~/.zfunc/_kaanal

# fish
kaanal completions fish > ~/.config/fish/completions/kaanal.fish

# PowerShell (add to $PROFILE)
kaanal completions powershell | Out-String | Invoke-Expression

# elvish (add to ~/.config/elvish/rc.elv)
eval (kaanal completions elvish | slurp)
```

Static completions cover subcommands, flags, and fixed values (including hidden flags like `--detached`). Open a new shell afterwards.
