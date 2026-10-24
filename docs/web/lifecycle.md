# Starting, stopping and exit codes

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
UI:    http://localhost:8080/__kaanal/ui#token=e494a4b6367a8c22d1acea651459c806e484be471aa6962d3a2c2826ff647eb6
Log: C:\work\shop\.kaanal\kaanal.log
Stop: kaanal down
```

1. **Checks the config here**, exactly as `kaanal up` does. A bad `kaanal.yaml` is a `file:line:col` error and exit 1, before anything is created or started.
2. **Takes the start lock** (`.kaanal/start.lock`), so two `up -d` in one folder run one after the other.
3. **Looks for a running instance.** If one is up and answering its health check, it prints that instance's lines and `already running (pid N)`, and exits 0. One that holds the lock but does not answer yet (still starting, or in the middle of a shutdown) is waited for, within `--wait`: when it goes away a fresh one is started, and if it is still unresponsive at the deadline the command fails with `kaanal is running but not answering`. A leftover `run.json` of a dead instance is removed (see [stale state](#stale-state)).
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
UI:    http://localhost:8080/__kaanal/ui#token=e494...
already running (pid 29228)
```

That makes `up -d` safe to put at the top of every test script. If the running instance is still starting, the second command waits up to 5 s for it to publish its state, then fails with `kaanal is starting or its state file is missing` (exit 1).

**Simultaneous starts.** Five `kaanal up -d` launched at the same instant start exactly one instance; the other four wait their turn on the start lock, find it running, print `already running` and exit 0, and none of them rotates away the log of the live instance. A command that waits on the start lock is not limited by its own `--wait`: it waits for the one ahead of it, which is limited by that command's `--wait` plus its checks.

**Non-loopback `--host`.** `kaanal up -d --host 0.0.0.0` prints the network-exposure warning in your terminal, and the background process also writes it to the log. See the [safety notes](user-guide.md#safety-notes).

### The log

`.kaanal/kaanal.log` is the background process's standard output and error: the startup lines (`Mocks:`, `UI:` without the token, `Auth:`, `Seed:`), warnings, and a panic message if there ever is one. Read it with any pager. It is replaced at each `up -d` (the previous one stays as `kaanal.log.1`, one generation only). If the old log cannot be renamed (on Windows, an editor holding it open), `up -d` warns and truncates it instead of refusing to start.

**What is never in it:** the token (the background process prints the `UI:` line without `#token=`; the token lives in `.kaanal/run.json`, in a folder only you can read, and in your terminal), request bodies, headers or any per-request line. The [request log](observability.md) is in memory and dies with the process; this file is process output, not an access log. Foreground `kaanal up` writes no file.

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
2. **Asks the instance to stop** with [`POST /__kaanal/shutdown`](control-api.md#post-__kaanalshutdown) and the budget you gave. If the instance is still starting, it waits up to 5 s for it to publish its state first.
3. **Waits for the process to be gone** for the budget plus 2 s (it watches the instance lock, which the operating system releases when the process ends, however it ends). Then it prints `stopped`, exit 0.
4. **Force-kills only as a last resort**: when the instance did not go away in time, or its control API answered with an error (a wrong token, a 5xx: no drain was started). It never kills on a guess:
   - it re-reads `run.json` and the lock right before checking and again right before the kill, and only goes on if it is still the same instance (same token and pid, lock held); an instance that ends by itself in the meantime is a plain `stopped`;
   - it checks that the pid really is a kaanal process (below);
   - it never targets its own pid, and a `run.json` without a pid is never a reason to kill anything.

   A kill prints `stopped (killed pid 4120 after 5s)`.

The refusals are exit 1 with the reason, and nothing is killed: `kaanal did not stop, and pid 4120 is not a kaanal process (ping.exe); not killing it`, `... could not be verified as kaanal; not killing it: ...`, `kaanal instance changed while stopping; not killing`.

**Which pids count as kaanal.** The process's image name must be `kaanal` (`kaanal.exe` on Windows), or the file name of the running `kaanal` itself, ignoring case. So a renamed or hashed copy (`kaanal-linux-x64`) can stop daemons that it started, and never anyone else's process. The check uses the operating system's tools by full path, so a program placed earlier on your `PATH` is never run: `tasklist` and `taskkill` from `System32` on Windows; `ps` and `kill` in `/bin` or `/usr/bin` on Linux and macOS. If they are in neither (NixOS keeps them elsewhere), the pid cannot be verified and `down` refuses to kill; the graceful path does not need them. On Linux `ps` shows only the first 15 characters of the name, and the check compares that many; a longer renamed binary is matched on its first 15.

**A second `down` while a drain is running.** A drain started with a long budget (`down --timeout 60s`) is not extended by a later command. The server answers a second shutdown request with the time left of the drain already running, and `down` waits for the longer of its own `--timeout` and that time left, plus 2 s, before it considers a force-kill. So a second `down` does not cut an earlier, longer drain short; it only waits for it.

**Exit codes of `down`:** 0 stopped or not running, 1 it could not stop it (or would not kill), 64 a bad `--timeout`. It never exits 2.

### Remote down

With an endpoint (`kaanal --endpoint http://host:8080 down`, token from `KAANAL_TOKEN`), `down` stops an instance that is not in this folder, for example one in a container or on another machine. It does only what it can do over HTTP:

1. `POST /__kaanal/shutdown` with your `--timeout` as the budget.
2. Polls `/__kaanal/health` until the connection is refused, reset or times out while connecting, for the budget plus 2 s. Then it prints `stopped`, exit 0.

It never reads `.kaanal/run.json`, the lock or a pid, and it never kills anything: an instance that still answers when the time is up is exit 1 (`http://host:8080 is still answering after 7s; not killing a remote instance`). No answer to the shutdown request is exit 1 as well (`cannot reach ...`), including when nothing is running there, so unlike the local `down` it is not idempotent.

`stopped` here means "stopped answering", not "the process exited". If a firewall silently drops packets after the shutdown was accepted, the connect timeout looks the same as a gone instance and `down` can print `stopped` while the process is still draining. Use a local `down` on that host when you need the certainty.

`up` and `up -d` refuse an endpoint (exit 64). The flags are described in the [user guide](user-guide.md#global-flags).

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

`kaanal logs -f` follows this rule too. When the stream ends after a `shutdown` event (a `down` or a signal), it waits for the process to exit and exits 2 with `kaanal stopped (the server shut down)`. When the stream ends without one (the instance was killed, or the drain budget was so small that the event was never written) it waits up to 2 s for the lock to go, then exits 2 if the instance is gone, or 1 if the lock is still held (`kaanal is running (pid N) but its control API did not answer: the event stream closed`).

## Signals

| Platform     | Signals that stop `kaanal up`                                   |
| ------------ | --------------------------------------------------------------- |
| Linux, macOS | `SIGTERM` (`kill`, systemd, a CI runner), `SIGINT` (Ctrl+C)     |
| Windows      | Ctrl+C, Ctrl+Break, closing the console window, system shutdown |

All of them do the same: [drain](#the-drain) for 5 s, remove `run.json`, exit 0. The 5 s is fixed for signals; use `kaanal down --timeout` for another budget. The handlers are installed as soon as the runtime starts, before anything is published (`run.json`, the URLs); a signal that arrives from then on is queued and handled once the server is up. A signal during config loading or key generation, before that point, takes the default action and ends the process; nothing has been published by then, so nothing is left behind.

- **A hard kill** (`SIGKILL`, `taskkill /F`, a crashed machine) skips all of this and leaves [stale state](#stale-state), which the next command cleans up.
- **A detached Windows daemon has no console**, so it never sees Ctrl+C or a console close. Stop it with `kaanal down` (or `POST /__kaanal/shutdown`). On Linux and macOS the daemon ignores `SIGHUP`, as `nohup` does, so closing the terminal does not kill it, and it is in its own process group, so Ctrl+C in that terminal does not reach it either.
- **A second Ctrl+C while draining** does nothing on Linux and macOS (the signal is swallowed and the drain carries on). On Windows it terminates the process at once, leaving stale state.

## The drain

Stopping (a signal, `down`, or the API) starts a **drain** with a budget (5 s unless told otherwise):

- The server stops accepting connections at once. A new connection is refused, even if the budget is 60 s.
- Requests already in flight carry on and finish normally, and their responses are delivered.
- At the end of the budget every connection still open is cut. This is what a client sees when a server goes away mid-request: the connection closes with no response. A request held by `hang` chaos, a 30 s latency or a `loading` pin is cut this way. If nothing is in flight the process exits as soon as the drain starts.
- The process then removes `run.json` (only if it still holds this instance's token, so an instance started in the meantime is never touched) and exits 0.

**Event streams.** Open [`/__kaanal/events`](control-api.md#get-__kaanalevents) streams (a status page tab, `kaanal logs -f`) would otherwise hold the drain open for the whole budget. So when the drain starts, each of them receives one last message and ends:

```
event: shutdown
data: {"timeout_ms":2999}
```

`timeout_ms` is the time left in the drain. The consequence: the events of requests that are **cut** at the end of the budget (the `client-gone` entries) do not reach a follower, because its stream is already closed. The request log lives in the process and is gone either way.

## Windows: pipes and `kaanal-winhandle`

`$(kaanal up -d)`, a CI step and a Node `execFileSync` all read the command's output until it closes. A background process that inherited the command's output pipe would keep it open for ever and the step would hang. Windows makes this easy to get wrong: Rust always starts child processes with handle inheritance on, so a plain spawn would hand the daemon the caller's pipes.

`up -d` avoids it by marking its three standard handles (stdin, stdout, stderr) non-inheritable for the instant of the spawn and restoring them afterwards. That is one Win32 call (`SetHandleInformation`), which needs `unsafe` code; Kaanal forbids `unsafe` everywhere except one tiny crate for exactly this, `kaanal-winhandle` (rule R1.46, see [the per-file page](../source/files/kaanal-winhandle/lib.md)). On other platforms there is nothing to do.

Limits: only those three handles are covered. If your own program has passed an extra inheritable handle to `kaanal up -d` (an extra pipe, an open file you made inheritable), the daemon can inherit that one. The daemon is also started outside the caller's job object when Windows allows it (so a terminal or IDE that kills its job on close does not take the daemon with it); when the job forbids that, it starts inside the job and ends with it.

## Exit codes

| Code | Meaning                                                                                                                                                                                                                                                                              |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 0    | Success. Also: `down` when it stopped the instance or nothing was running, `up -d` when it was already running, `--help` and `--version`.                                                                                                                                            |
| 1    | Error: a bad config, a startup failure, a value the server refused (HTTP 400), an instance that is running but does not answer or is still starting, a `down` that could not or would not kill. In remote mode (`--endpoint`): cannot reach the instance, or the token was rejected. |
| 2    | Not running: a command that needs a local instance found none, or only stale state. `down` and `up -d` never return it, and neither does any command in remote mode.                                                                                                                 |
| 3    | Reserved for assertion failures. No command returns it yet.                                                                                                                                                                                                                          |
| 64   | Usage error: the command line is wrong, found before any request was sent. In remote mode: a bad `--endpoint`, a missing token, `--token` without an endpoint, `up` with an endpoint.                                                                                                |

**What is a 64.** Everything clap rejects (an unknown flag or subcommand, a missing argument, flags that exclude each other, a bad `--timeout`, `--wait` without `-d`) and every other bad argument the CLI can see for itself before it sends anything: a bad latency or rate, `--ttl`, `--state`, `--delay`, `--from`, a `-n` outside 1 to 1000, an empty `--scope`, a bad `--since` or `--status` (with or without `-f`), `chaos set` with nothing to set, a bad scenario name or file name. 64 is `EX_USAGE` from `sysexits.h`. (Before this release these exited 1, and clap's own errors would have exited 2, which means "not running" here.)

**Remote mode is never a 2.** Exit 2 means "no instance in this folder", which a remote command cannot know. With `--endpoint`, a refused, reset or timed-out connection is exit 1 (`cannot reach http://host:8080: ...`) and a rejected token is exit 1 (`token rejected`). Misusing the flags is a 64: a URL that is not `http://HOST:PORT` (https, a path, a user or password, no port), `--endpoint` without a token (`set KAANAL_TOKEN`), the `--token` flag without an endpoint, or `up` / `up -d` while an endpoint is set (flag or `KAANAL_ENDPOINT`).

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
- The daemon log holds process output only. There is no access log and no request data on disk; the [request log](observability.md) is a 1000-entry ring in memory.
- `.kaanal/lock` and `.kaanal/start.lock` are empty files that exist to be locked. They are safe to delete when nothing runs, and are re-created on demand.

> Contributor pointer: the pieces are `crates/kaanal/src/instance/lifecycle/` (liveness, stopping, detaching) and `up`, `up_detached` and `down` in `main.rs`; the server side is `crates/kaanal-server/src/shutdown.rs`. See [../source/files/kaanal/lifecycle.md](../source/files/kaanal/lifecycle.md).
