# `crates/kaanal-winhandle/src/lib.rs`

The one crate in the workspace that may contain `unsafe` code (rule R1.46). It exists for a single Win32 call.

## Why it exists

Rust's `std::process::Command` always creates processes with `bInheritHandles = TRUE`. A daemon started by `kaanal up -d` would therefore inherit the caller's stdin, stdout and stderr handles. When those are pipes (`$(kaanal up -d)`, a CI step, Node's `execFileSync`), the daemon keeps the write end open for as long as it lives and the reader never sees end-of-file. The fix is to mark the three standard handles non-inheritable around the spawn, which is `SetHandleInformation`, which is `unsafe`. The workspace forbids `unsafe` (`unsafe_code` in `[workspace.lints]`), so the call lives here, alone, instead of loosening the rule everywhere.

User-facing note: [../../../web/lifecycle.md](../../../web/lifecycle.md#windows-pipes-and-kaanal-winhandle).

## Items

| Item                                | Kind   | What it does                                                                                                                                                                                                                                                                                                                                                                                             |
| ----------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `without_inheritable_stdio(f) -> T` | pub fn | Runs `f` with this process's stdin, stdout and stderr handles non-inheritable, then restores their flags (also if `f` panics, through a `Drop` guard). Handles that are null, invalid, already non-inheritable or unreadable are left alone, and any failure is ignored (best effort). A child spawned inside `f` still gets its own stdio through explicit `Stdio` values, which std duplicates for it. |
| `imp` (Windows)                     | mod    | `GetStdHandle` for each of the three, `GetHandleInformation` to see the inherit bit, `SetHandleInformation` to clear it, the list of cleared handles kept in `Restore`. Each `unsafe` block has a `SAFETY:` comment. The module carries `#[expect(unsafe_code, reason = "R1.46: the one SetHandleInformation call")]`.                                                                                   |
| `imp` (other platforms)             | mod    | Calls `f()` and nothing else.                                                                                                                                                                                                                                                                                                                                                                            |

## Limits

- Only the **three standard handles** are covered. A `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` would cover every handle, but needs far more `unsafe` and is out of scope under R1.46. An extra inheritable handle that the caller passed on (a pipe, a file) can still reach the daemon.
- Not safe to call from two threads at once: the flags are process-wide, and the second call would restore the first one's cleared state. The only caller is `lifecycle::spawn_detached`, on the main thread.
- `windows-sys` is a Windows-only dependency (features `Win32_Foundation` and `Win32_System_Console`).

## Lints

`Cargo.toml` sets `unsafe_code = "deny"` and a copy of the workspace lint table (a crate cannot both inherit the workspace table and override one lint). Keep the copy in step with `[workspace.lints]`. The single `#[expect(unsafe_code)]` on the Windows module is the only lift, so any other `unsafe` here fails the build.

## Tests

None in the crate: the call has no logic of its own. The behaviour is proven end to end by `piped_up_d_returns_while_the_daemon_keeps_running` and `up_d_returns_after_health_and_run_json_has_a_live_port` in [../../tests.md](../../tests.md#crateskaanaltestsclirs--binary-end-to-end).
