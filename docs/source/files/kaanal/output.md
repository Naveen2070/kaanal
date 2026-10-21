# `crates/kaanal/src/output.rs` (private)

The global `--json` switch: one process-wide bit, set once in `main`, read by every command that prints.

## What it does

`main` calls `set_json(cli.json || env_on())` before dispatch. A command that prints a result builds the `serde_json::Value` it got from the control API and hands it to `emit` together with a closure for the text form. In text mode the closure runs; in JSON mode the value is printed as one compact line (a `Null`, which is what a 204 answer becomes, prints nothing). Errors never go through here: they stay text on stderr with their exit codes, so `--json` changes only what is printed, never what a command does (spec J1-J3).

## Items

| Item             | What it does                                                                                                                                                     |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `JSON`           | The `AtomicBool` holding the switch (relaxed ordering: it is written once before any command runs).                                                              |
| `set_json(on)`   | Turns JSON output on or off. Called once from `main`.                                                                                                            |
| `json_on()`      | Whether JSON output is on. `logs` and `trace` read it to choose between the text line and the entry JSON; only `status` uses it to skip fetching text-only data. |
| `env_on()`       | `KAANAL_JSON` counts as the flag when it is `1` or `true`; empty or anything else is off.                                                                        |
| `emit(v, human)` | Text mode: runs `human`. JSON mode: prints `v` as one line through `out_line` (so a closed pipe exits 0 quietly), or nothing when `v` is `Null`.                 |

Callers: `status`, `pins`, `scenario ...`, `chaos set|clear`, `cycle` and `reset` call `emit`; `logs` and `trace` read `json_on()` directly (in [main.md](main.md)). `completions` and `validate` ignore the switch.

## Tests

Covered end to end in [tests.md](../../tests.md#json-remote-mode-and-completions): `json_flag_works_before_and_after_the_subcommand`, `json_env_switches_it_on_and_empty_env_is_off`, `every_client_command_honours_json` and the rest of the `json_*` tests. The module has no unit tests of its own.
