---
title: "Starting, stopping and exit codes"
sidebar:
  order: 2
---

A CI job or a test setup needs three things from a mock backend: start it and know when it is ready, stop it without leaving a mess, and tell "I typed it wrong" from "it failed" from "nothing is running" by the exit code alone. `kaanal up -d` starts Kaanal in the background and returns when it answers; `kaanal down` stops it; the exit codes are the same for every command.

```bash
kaanal up -d      # returns when the server answers /__kaanal/health
npm test
kaanal down       # drains, stops, exit 0 (also when nothing is running)
```

## `kaanal up`: foreground and background

|                         | `kaanal up`                                     | `kaanal up -d`                                      |
| ----------------------- | ----------------------------------------------- | --------------------------------------------------- |
| Runs                    | In your terminal, until you stop it             | In the background; the command returns              |
| Output                  | The URLs, then silence                          | The URLs after the server answers, then it exits    |
| The server's own output | Your terminal                                   | `.kaanal/kaanal.log`                                |
| A second one            | Fails: `kaanal is already running on port 8080` | Prints the running instance and exits 0             |
| Stop it with            | Ctrl+C, `SIGTERM`, `kaanal down`                | `kaanal down` (or `SIGTERM` to the pid in run.json) |

### `up -d`, step by step

```
$ kaanal up -d
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/studio?mode=admin#token=e494a4b6367a8c22d1acea651459c806e484be471aa6962d3a2c2826ff647eb6
Log: C:\work\shop\.kaanal\kaanal.log
Stop: kaanal down
```

1. **Checks the config here**, exactly as `kaanal up` does. A bad `kaanal.yaml` is a `file:line:col` error and exit 1, before anything is created or started.
2. **Takes the start lock** (`.kaanal/start.lock`), so two `up -d` in one folder run one after the other.
3. **Looks for a running instance.**
   - If one answers its health check, prints its URLs and `already running (pid N)` and exits `0`.
   - If one holds the lock without answering yet (starting or shutting down), waits up to `--wait`, then either starts a fresh one or fails with `kaanal is running but not answering`.
   - Removes any leftover `run.json` from a dead instance (see [stale state](#stale-state)).
4. **Rotates the log**: the old `.kaanal/kaanal.log` becomes `.kaanal/kaanal.log.1` (replacing the previous one) and a new, empty `kaanal.log` is created.
5. **Starts a copy of itself** detached from your terminal, with its output in the log.
6. **Waits** until that process has written its own `run.json`, holds the instance lock, and answers `GET /__kaanal/health` with 200. It checks every 100 ms.
7. **Prints** the `Mocks:` and `UI:` lines (and `Auth:`, `Seed:` and warnings when there are any: the same lines `kaanal up` prints), then `Log:` with the full path of the log and `Stop: kaanal down`. Exit 0.

The `UI:` line carries the token (`#token=...`), as with `kaanal up`. It goes to your terminal and nowhere else: if your CI log is public, treat that line as a secret. The token only works for this one run.

`--wait` is how long step 6 may take: `kaanal up -d --wait 30s`. The default is 15 s; any value from `1ms` to `1h` is accepted (anything else is a [usage error](#exit-codes)). `--wait` only exists together with `-d`.

**When it does not start.** The command exits 1 with the reason and the last 20 lines of the log:

```
kaanal did not start: the background process exited; last lines of C:\work\shop\.kaanal\kaanal.log:
error: no free port in 8080..=8100 ...
```

A process that is still not ready at the deadline is asked to shut down and, if it ignores that, killed: `up -d` never leaves a half-started process behind. The message then says `no answer within 15s`. An empty log shows as `(log is empty)`.

**Healthy just after the deadline.** If the process answers in the instant between the last check and the decision to give up, `up -d` keeps it: it prints the usual lines plus `note: kaanal answered just after the 15s wait; leaving it running` on stderr, and exits 0. The instance is usable, so killing it would only make the next attempt slower.

**Starting twice.** A second `up -d` in a folder with a running instance changes nothing and exits 0:

```
$ kaanal up -d
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/studio?mode=admin#token=e494...
already running (pid 29228)
```

That makes `up -d` safe to put at the top of every test script. If the running instance is still starting, the second command waits up to 5 s for it to publish its state, then fails with `kaanal is starting or its state file is missing` (exit 1).

**Simultaneous starts.** Five `kaanal up -d` calls launched together start one instance. The other four wait on the start lock, see it running, print `already running`, and exit `0` without rotating the live log.

Waiting on the start lock is not bounded by your own `--wait`: it waits for the command ahead of it, which is bounded by that command's `--wait` plus its checks.

**Non-loopback `--host`.** `kaanal up -d --host 0.0.0.0` prints the network-exposure warning in your terminal, and the background process also writes it to the log. See the [safety notes](/docs/how-it-works/safety/).

### The log

`.kaanal/kaanal.log` captures the daemon's stdout/stderr (`Mocks:`, `UI:` without token, `Auth:`, `Seed:`, warnings, panics). Each `up -d` rotates the old file to `kaanal.log.1` (truncating with a warning on Windows if an editor holds it open).

**What is never in it:** the token (kept in `.kaanal/run.json` and printed to your terminal), request bodies, headers, or per-request lines. The [request log](/docs/guides/observe/observability/) lives in memory; foreground `kaanal up` writes no file.

## `kaanal down`

```
$ kaanal down
stopped
$ kaanal down
kaanal is not running
```

`down` is idempotent: it exits 0 when nothing is running, so it is safe in a cleanup step that runs whether or not the start worked.

```bash
kaanal down                  # drain for up to 5 s
kaanal down --timeout 30s    # let slow in-flight requests finish
```

`--timeout` is the drain budget, `1ms` to `300s`, default `5s`.

What it does:

1. **No instance?** If nothing holds the instance lock, it removes any leftover `run.json` and says so (`kaanal is not running (removed stale state from pid 4120)`), exit 0.
2. **Asks the instance to stop** with [`POST /__kaanal/shutdown`](/docs/reference/control-api/shutdown-and-ui/#post-__kaanalshutdown) and the budget you gave. If the instance is still starting, it waits up to 5 s for it to publish its state first.
3. **Waits for the process to be gone** for the budget plus 2 s (it watches the instance lock, which the operating system releases when the process ends, however it ends). Then it prints `stopped`, exit 0.
4. **Force-kills only as a last resort**: when the instance did not go away in time, or its control API answered with an error (a wrong token, a 5xx: no drain was started). It never kills on a guess:
   - it re-reads `run.json` and the lock right before checking and again right before the kill, and only goes on if it is still the same instance (same token and pid, lock held); an instance that ends by itself in the meantime is a plain `stopped`;
   - it checks that the pid really is a kaanal process (below);
   - it never targets its own pid, and a `run.json` without a pid is never a reason to kill anything.

   A kill prints `stopped (killed pid 4120 after 5s)`.

The refusals are exit 1 with the reason, and nothing is killed: `kaanal did not stop, and pid 4120 is not a kaanal process (ping.exe); not killing it`, `... could not be verified as kaanal; not killing it: ...`, `kaanal instance changed while stopping; not killing`.

**Which pids count as kaanal.** The image name must be `kaanal` (`kaanal.exe` on Windows) or the running binary's own filename (case-insensitive), so renamed copies (`kaanal-linux-x64`) only stop daemons they started.

- **Verified via OS tools by full path** (never `PATH`): `tasklist`/`taskkill` in `System32` on Windows; `ps`/`kill` in `/bin` or `/usr/bin` on Linux/macOS.
- If missing (e.g. NixOS), PID verification fails and `down` refuses to kill (graceful drain still works). On Linux, `ps` truncates names to 15 characters, so comparisons use the first 15.

**A second `down` while draining.** A running drain (e.g. `down --timeout 60s`) is not shortened or extended by a later `down`; the server returns the remaining drain time, and `down` waits for `max(own --timeout, remaining) + 2s` before considering a kill.

**Exit codes of `down`:** 0 stopped or not running, 1 it could not stop it (or would not kill), 64 a bad `--timeout`. It never exits 2.

### Remote down

With an endpoint (`kaanal --endpoint http://host:8080 down`, token from `KAANAL_TOKEN`), `down` stops an instance that is not in this folder, for example one in a container or on another machine. It does only what it can do over HTTP:

1. `POST /__kaanal/shutdown` with your `--timeout` as the budget.
2. Polls `/__kaanal/health` until the connection is refused, reset or times out while connecting, for the budget plus 2 s. Then it prints `stopped`, exit 0.

It never reads `.kaanal/run.json`, the lock or a pid, and it never kills anything: an instance that still answers when the time is up is exit 1 (`http://host:8080 is still answering after 7s; not killing a remote instance`). No answer to the shutdown request is exit 1 as well (`cannot reach ...`), including when nothing is running there, so unlike the local `down` it is not idempotent.

`stopped` here means "stopped answering", not "the process exited". If a firewall silently drops packets after the shutdown was accepted, the connect timeout looks the same as a gone instance and `down` can print `stopped` while the process is still draining. Use a local `down` on that host when you need the certainty.

`up` and `up -d` refuse an endpoint (exit 64). The flags are described in the [user guide](/docs/reference/cli/#global-flags).

## Stale state

A crash, `kill -9`, a power cut or `taskkill /F` leaves `.kaanal/run.json` behind. Kaanal never trusts that file by itself. Whether an instance is alive is decided by the **instance lock**, `.kaanal/lock`: the running process holds it for as long as it lives and the operating system drops it the moment the process ends, however it ends. A `run.json` whose lock is free is stale.

| What you run                                  | Lock                        | What happens                                                                                                               |
| --------------------------------------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `status`, `chaos`, `logs`, `reset`, ... (any) | free, `run.json` there      | `kaanal is not running (removed stale state from a stopped instance)`, file removed, **exit 2**                            |
| the same                                      | free, no `run.json`         | `kaanal is not running (no .kaanal/run.json)`, **exit 2**                                                                  |
| the same                                      | free, `run.json` unreadable | `kaanal is not running (removed unreadable state from a stopped instance)`, **exit 2**                                     |
| the same                                      | held, `run.json` there      | works; if the control API does not answer: `kaanal is running (pid N) but its control API did not answer: ...`, **exit 1** |
| the same                                      | held, no `run.json` for 5 s | `kaanal is starting or its state file is missing`, **exit 1**                                                              |
| `down`                                        | free                        | removes the file, says so, **exit 0**                                                                                      |
| `up`, `up -d`                                 | free                        | removes the file and starts normally                                                                                       |

```
$ kaanal status        # after the daemon was killed with taskkill /F
kaanal is not running (removed stale state from a stopped instance)
$ echo $?
2
```

Nothing is ever deleted while the lock is held, and a lock that cannot be tested at all (a network drive without file locking) is an error (exit 1), never "stale". A request that gets no answer is classified the same way: lock free means the instance is gone (exit 2, `kaanal is not running on 127.0.0.1:8080: ...`), lock held means it is alive but busy or wedged (exit 1).

`kaanal logs -f` follows the same rule:

- After a `shutdown` event (`down` or signal), it waits for exit and exits `2` (`kaanal stopped (the server shut down)`).
- Without one (killed or tiny drain budget), it waits up to `2s` for the lock to clear, exiting `2` if gone or `1` if still held.

## Signals

| Platform     | Signals that stop `kaanal up`                                   |
| ------------ | --------------------------------------------------------------- |
| Linux, macOS | `SIGTERM` (`kill`, systemd, a CI runner), `SIGINT` (Ctrl+C)     |
| Windows      | Ctrl+C, Ctrl+Break, closing the console window, system shutdown |

All of them drain for a fixed `5s`, remove `run.json`, and exit `0` (use `kaanal down --timeout` for another budget). Handlers install right as the runtime starts; signals during earlier config/key setup take the default action and exit before anything is published.

- **A hard kill** (`SIGKILL`, `taskkill /F`, a crashed machine) skips all of this and leaves [stale state](#stale-state), which the next command cleans up.
- **A detached Windows daemon has no console**, so it never sees Ctrl+C or a console close. Stop it with `kaanal down` (or `POST /__kaanal/shutdown`). On Linux and macOS the daemon ignores `SIGHUP`, as `nohup` does, so closing the terminal does not kill it, and it is in its own process group, so Ctrl+C in that terminal does not reach it either.
- **A second Ctrl+C while draining** does nothing on Linux and macOS (the signal is swallowed and the drain carries on). On Windows it terminates the process at once, leaving stale state.

## The drain

Stopping (a signal, `down`, or the API) starts a **drain** with a budget (5 s unless told otherwise):

- The server stops accepting connections at once. A new connection is refused, even if the budget is 60 s.
- Requests already in flight carry on and finish normally, and their responses are delivered.
- At the end of the budget every connection still open is cut. This is what a client sees when a server goes away mid-request: the connection closes with no response. A request held by `hang` chaos, a 30 s latency or a `loading` pin is cut this way. If nothing is in flight the process exits as soon as the drain starts.
- The process then removes `run.json` (only if it still holds this instance's token, so an instance started in the meantime is never touched) and exits 0.

**Event streams.** Open [`/__kaanal/events`](/docs/reference/control-api/live-and-events/#get-__kaanalevents) streams (a Studio tab, `kaanal logs -f`) would otherwise hold the drain open for the whole budget. So when the drain starts, each of them receives one last message and ends:

```
event: shutdown
data: {"timeout_ms":2999}
```

`timeout_ms` is the time left in the drain. The consequence: the events of requests that are **cut** at the end of the budget (the `client-gone` entries) do not reach a follower, because its stream is already closed. The request log lives in the process and is gone either way.

## Windows: pipes and `kaanal-winhandle`

`$(kaanal up -d)`, a CI step and a Node `execFileSync` all read the command's output until it closes. A background process that inherited the command's output pipe would keep it open for ever and the step would hang. Windows makes this easy to get wrong: Rust always starts child processes with handle inheritance on, so a plain spawn would hand the daemon the caller's pipes.

`up -d` avoids this by temporarily marking stdin, stdout, and stderr non-inheritable during spawn (`SetHandleInformation` in the isolated `kaanal-winhandle` crate, the only place `unsafe` is allowed; see [kaanal-winhandle](https://github.com/Naveen2070/kaanal/blob/main/docs/source/files/kaanal-winhandle/lib.md)). Other platforms need no special handling.

Only those three standard handles are covered; extra inheritable handles passed by a parent can still be inherited. When Windows allows, the daemon also starts outside the caller's job object so closing a terminal or IDE does not kill it.

## Exit codes

| Code | Meaning                                                                                                                                                                                                                                                                              |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 0    | Success. Also: `down` when it stopped the instance or nothing was running, `up -d` when it was already running, `--help` and `--version`.                                                                                                                                            |
| 1    | Error: a bad config, a startup failure, a value the server refused (HTTP 400), an instance that is running but does not answer or is still starting, a `down` that could not or would not kill. In remote mode (`--endpoint`): cannot reach the instance, or the token was rejected. |
| 2    | Not running: a command that needs a local instance found none, or only stale state. `down` and `up -d` never return it, and neither does any command in remote mode.                                                                                                                 |
| 3    | Reserved for assertion failures. No command returns it yet.                                                                                                                                                                                                                          |
| 64   | Usage error: the command line is wrong, found before any request was sent. In remote mode: a bad `--endpoint`, a missing token, `--token` without an endpoint, `up` with an endpoint.                                                                                                |

**What is a 64 (`EX_USAGE`).** All clap validation errors plus local CLI checks before sending a request: bad durations/rates, `--ttl`, `--state`, `--delay`, `--from`, `-n` outside `1..1000`, bad `--since`/`--status`, empty `chaos set`, or invalid scenario/file names.

**Remote mode is never a 2.** Exit `2` means "no local instance in this folder". With `--endpoint`, connection failures and rejected tokens exit `1`. Flag misuse (non-`http://HOST:PORT`, missing token, `--token` without `--endpoint`, or `up`/`up -d` with an endpoint) exits `64`.

**What stays a 1.** A value that parses on your side and that the server then refuses. `kaanal trace "bad id!"` is an HTTP 400 from the server, so exit 1.

A script can then branch on the code alone:

```bash
kaanal chaos set db --latency 300ms
case $? in
  0)  ;;
  2)  echo "start kaanal first" ;;
  64) echo "typo in the command" ;;
  *)  echo "kaanal failed" ;;
esac
```

## CI recipes

### GitHub Actions

```yaml
jobs:
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install kaanal # or download a release binary
      - name: Start Kaanal
        run: kaanal up -d --wait 30s
      - run: npm ci && npm test
      - name: Kaanal log
        if: failure()
        run: cat .kaanal/kaanal.log
      - name: Stop Kaanal
        if: always()
        run: kaanal down
```

`up -d` fails the step (exit 1) with the log tail when the server does not come up, so nothing after it runs against a dead backend. `down` after `always()` costs nothing when the server never started.

### Playwright

Start it in `globalSetup` and return the teardown:

```ts
// global-setup.ts
import { execFileSync } from 'node:child_process';

export default async function globalSetup() {
  execFileSync('kaanal', ['up', '-d', '--wait', '30s'], { stdio: 'inherit' });
  return async () => {
    execFileSync('kaanal', ['down'], { stdio: 'inherit' }); // teardown, also when tests failed
  };
}
```

Or let Playwright own a foreground process, with the health endpoint (no token needed) as its readiness check:

```ts
// playwright.config.ts
export default defineConfig({
  webServer: {
    command: 'kaanal up',
    url: 'http://localhost:8080/__kaanal/health',
    reuseExistingServer: !process.env.CI,
  },
});
```

If Playwright ends the process hard, the next start removes the [stale state](#stale-state).

### A shell script

```bash
kaanal up -d || exit 1
trap 'kaanal down' EXIT     # runs on success, failure and Ctrl+C
npm test
```

### Waiting for a Kaanal someone else started

`up -d` already waits. For a Kaanal started by another process, poll the health endpoint, which needs no token:

```bash
until curl -fs http://localhost:8080/__kaanal/health >/dev/null; do sleep 0.2; done
```

## What is not stored

- The token is never written to the log, the `Log:` line or any file except `.kaanal/run.json`. `run.json` also holds the port, the pid and a `startup` object (`auth`, `seed`, `warnings`: the lines `up` printed, so a second `up -d` can print them); the folder is readable only by you.
- The daemon log holds process output only. There is no access log and no request data on disk; the [request log](/docs/guides/observe/observability/) is a 1000-entry ring in memory.
- `.kaanal/lock` and `.kaanal/start.lock` are empty files that exist to be locked. They are safe to delete when nothing runs, and are re-created on demand.

> Contributor pointer: the pieces are `crates/kaanal/src/instance/lifecycle/` (liveness, stopping, detaching) and `up`, `up_detached` and `down` in `main.rs`; the server side is `crates/kaanal-server/src/shutdown.rs`. See [../source/files/kaanal/lifecycle.md](https://github.com/Naveen2070/kaanal/blob/main/docs/source/files/kaanal/lifecycle.md).
