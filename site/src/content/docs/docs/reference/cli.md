---
title: "CLI"
sidebar:
  order: 2
---

All commands that talk to a running instance read `.kaanal/run.json` in the current directory unless `--endpoint` is set.

## Lifecycle & authoring

- **`kaanal init [SPEC]`** `[--service NAME] [--port P] [--locale TAG] [--server-style S] [--yes] [--force]`
  Generates `kaanal.yaml` (and `.gitignore` entry) from an OpenAPI file or `http(s)` URL. Prompts for service, port, locale, and server style (`event-loop` or `thread-per-request`; defaults `app`, `8080`, `en-IN`, `event-loop`), writing estimated slots, queues, times, and `endpoints:` entries. See [init](/docs/guides/mock/init/).
- **`kaanal up`** `[-c FILE] [--port P] [--host IP]`
  Starts the mock server in the foreground (`Ctrl+C` or `SIGTERM` drains up to `5s`).
  - `-d [--wait 15s]`: daemonizes and returns once healthy with URLs (idempotent if already running, exit `0`). See [Lifecycle](/docs/guides/observe/lifecycle/).
  - `--blank`: starts empty with no system (cannot combine with `-d`; mocks return `404` until `draft apply`). See [Draft API](/docs/reference/control-api/draft/).
- **`kaanal studio`** `[-c FILE] [--port P] [--host IP] [--no-open]`
  Runs `up --blank` and opens Studio in your browser (`--no-open` prints the URL only).
- **`kaanal demo`** `[--port P] [--locale TAG] [--open] | --eject [DIR] [--force]`
  Runs the built-in demo shop with a chaos bar from `.kaanal/demo/` (`127.0.0.1` only), or ejects editable files with `--eject`. See [the demo](/docs/start/tour/).
- **`kaanal down`** `[--timeout 5s]`
  Stops the running instance (drains, then force-kills only a verified `kaanal` PID; exits `0` when nothing is running). With `--endpoint`, stops a remote instance: [remote down](/docs/guides/observe/lifecycle/#remote-down).
- **`kaanal validate`** `[-c FILE]`
  Validates config, topology, and OpenAPI specs without starting.
- **`kaanal schema`** `[config|dictionary|pack]`
  Prints the JSON Schema for `kaanal.yaml` (default), `data.yaml`, or `locales/<tag>.yaml`.
- **`kaanal endpoints`** `[-c FILE] [--write]`
  Prints skeleton `endpoints:` entries for uncovered spec routes (`--write` appends to the endpoints file).
- **`kaanal skeleton`** `[-o FILE] [--force]`
  Prints or writes a generated OpenAPI 3.0 file from the draft's `endpoints:`.
- **`kaanal sample`** `"GET /users/{id}" [--seed N]`
  Prints one generated response body (status and byte size go to stderr).

## Chaos, pins & draft

- **`kaanal status`** — Node table with effective chaos (scenarios + live) and active scenario names.
- **`kaanal chaos set`** `[node] [--route "GET /x"] [--replica N] [--latency 300ms] [--error-rate 10%] [--error 503] [--kind hang] [--down] [--for 30s]` — Merges live chaos fields (`--for` auto-reverts).
- **`kaanal chaos clear`** `[node] [--route "GET /x"] [--replica N]` — Clears live chaos on a target or all nodes (scenarios stay).
- **`kaanal node down`** `NODE [--replica N] [--for 30s]` — Marks a node `503` ([Timed chaos](/docs/guides/break/chaos-live/#timed-chaos)).
- **`kaanal node up`** `NODE [--replica N]` — Clears live `down` (exits `1` if a scenario/override still holds it down).
- **`kaanal node list`** — Prints the node status table with remaining timed-chaos countdowns.
- **`kaanal scenario`** `list|show NAME|activate NAME|deactivate NAME|set A B|clear|active|load FILE|save NAME` — Manages named chaos scenarios. See [Scenarios](/docs/guides/break/scenarios/).
- **`kaanal cycle`** `EP [--state S] [--status N] [--delay D] [--off]` — Steps or pins an endpoint (`loading`, `empty`, `error`, `slow`, `success`). `kaanal pins` lists active pins. See [Pins](/docs/guides/break/pins/).
- **`kaanal reset`** — Clears scenarios, live chaos, counters, rate-limit buckets, and the request log (request IDs continue).
- **`kaanal draft`** `show|set -f FILE|validate [-f FILE]|apply [--timeout T]|save [--confirm-loss] [--fix-scenarios remove]` — Inspects, edits, validates (`file:line:col: message`), applies live without restart, or saves `kaanal.yaml`. See [Draft API](/docs/reference/control-api/draft/).

## Simulation & observability

- **`kaanal sim run`** `-p NAME|-f FILE [--duration D] [--warmup D] [--seed N] [--scenario S] [--no-wait]` — Runs a seeded virtual-time simulation.
- **`kaanal sim sweep`** `-p NAME|-f FILE [--p99 1s] [--errors 1%] [--max-factor 8] [--seed N] [--scenario S] [--no-wait|--report FILE [--force]]` — Steps load until p99 or errors cross limits and reports the knee.
- **`kaanal sim plan`** `-p NAME|-f FILE --target 3x|500 [--p99 1s] [--errors 1%] [--budget 2.00] [--keep NODE]... [--max-steps N] [--seed N] [--scenario S] [--cheapest [--time 150s] [--run-cap 200] [--providers gcp,azure|none]] [--no-wait|--report FILE [-o plan.yaml]] [--force]` — Walks or searches (`--cheapest`) for a design holding the target load.
- **`kaanal sim list`**, **`sim show ID`**, **`sim report ID`** `[--json] [-o FILE [--force]]` — Lists runs, inspects a record, or outputs the Markdown/JSON report. See [Simulation](/docs/guides/simulate/simulation/).
- **`kaanal logs`** `[-n N] [-f] [--since D] [--status S] [--failed] [--route "GET /x/{id}"] [--scope NAME]` — Prints recent requests (`-f` follows live). See [Observability](/docs/guides/observe/observability/).
- **`kaanal trace`** `ID|last|CLIENT_ID` — Prints a per-hop waterfall for a request. See [Observability](/docs/guides/observe/observability/).
- **`kaanal completions`** `bash|zsh|fish|powershell|elvish` — Prints static shell completion scripts. See [Shell completions](#shell-completions).

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
