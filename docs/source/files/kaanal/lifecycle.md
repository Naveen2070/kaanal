# `crates/kaanal/src/instance/lifecycle/` (private)

Is an instance running, and how to start one in the background and stop one: the lock-based liveness check, the `run.json` reader, the stop path of `down`, the stop signals, and the pieces of `up -d`. Every function takes the state directory (`.kaanal`) as an argument, so the unit tests use a temp dir. User-facing description: [lifecycle guide](https://kaanal.naveenr.in/docs/guides/observe/lifecycle/).

## Liveness: the lock decides

`.kaanal/lock` is held (exclusively, for the life of the process) by a running `up`. The OS drops it when the process ends, however it ends. `probe` opens the file and tries to take it **shared**:

- success: nobody is running (`Probe::Free`, which keeps the shared lock in a `Guard` so a new `up` cannot take the exclusive lock between the probe and what the caller then does, such as deleting stale state);
- `WouldBlock`: an instance holds it (`Probe::Running`);
- any other error: an error (exit 1). It is never read as "stale": acting on a guess could delete a live instance's state.

Shared, because two probes must not make each other look "running"; `up` retries its exclusive lock for about 200 ms (`lock_at` in `main.rs`) so a probe never makes it report "already running". A `run.json` with no lock file at all gets a lock file created, so the caller still has a guard to hold.

## Modules

`lifecycle/mod.rs` keeps `STARTUP_WAIT` and re-exports the public items, so callers still write `lifecycle::read_run` and the like. The code is in:

| Module                                 | What it holds                                                                                           |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| [`liveness.rs`](lifecycle_liveness.md) | Is an instance running: the lock probe, the `run.json` reader and the stale-state cleanup.              |
| [`pid.rs`](lifecycle_pid.md)           | Telling whether a pid is a `kaanal` process (`tasklist` / `ps` parsing) and killing it.                 |
| [`stop.rs`](lifecycle_stop.md)         | Stopping an instance: waiting for it, the forced stop, and the stop signals of the server process.      |
| [`spawn.rs`](lifecycle_spawn.md)       | Starting the background instance for `up -d`: the log rotation and the detached spawn.                  |
| [`ready.rs`](lifecycle_ready.md)       | Waiting for a started instance to answer: polling, the unready settle, and the existing-instance check. |
| `tests.rs`                             | The unit tests (see Tests below).                                                                       |

## Tests

[../../tests.md](../../tests.md#crateskaanalsrcinstancelifecycletestsrs-unit--liveness-stopping-and-detaching). End-to-end: [../../tests.md](../../tests.md#crateskaanaltestsclirs--binary-end-to-end) (stale state, `down`, `up -d`).
