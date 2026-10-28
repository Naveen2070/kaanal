# `crates/kaanal/src/commands/draft.rs`

`kaanal draft`: show, replace, check, apply and save the running instance's draft (the text of its `kaanal.yaml`, kept apart from the running system until Apply). The overview of the commands is in [main.md](main.md); the HTTP contract is [docs/web/control-api.md](../../../web/control-api.md#draft).

## Functions

| Function | What it does |
| --- | --- |
| `draft(cmd)` | Dispatches `show`, `set`, `validate`, `apply`, `save`. |
| `show()` | `GET /draft`; prints the text exactly as stored (JSON mode: the whole answer). |
| `set(file)` | Reads the file and `PUT /draft` with its text. Nothing is applied. |
| `check(file)` | `POST /draft/validate`, with the file's text as the body (not stored) or none (the stored draft). `ok` returns the answer; otherwise the first diagnostic comes back as an error. |
| `first_problem(v, label)` | The first diagnostic as `file:line:col: message` through `config_err`, so it reads like `kaanal validate`. `label` is the `--file` path, or `draft` for the stored one; a problem in another file (a spec, the endpoints file) names that file. |
| `validate(file)` | `check`, then `ok`, the warnings (stderr) and the flagged scenario files. A problem exits 1. |
| `apply(timeout)` | Parses `--timeout` (default `10s`, at most `60s`; usage error otherwise), checks the stored draft first so a problem shows with its position, then `POST /draft/apply` through `call_for` with a budget of the timeout plus 15 s (the drain can use all of it). |
| `save(confirm_loss, fix)` | `POST /draft/save` with `confirm_loss` and `fix_scenarios` (`none`, `remove` or `rewrite`; `--fix-scenarios rewrite\|remove`). Prints `saved: kaanal.yaml` and, when files were deleted, `removed: ...`. A refusal (the file changed on disk, edits that dropped comments) is the server's message, exit 1; `--confirm-loss` goes ahead past the second. |
| `print_applied(v)` | `applied generation N`, `kept: N live chaos, N pins, scenarios: ...`, `dropped: ...` (live chaos labels, `scenario NAME`, `pin KEY`; only when something went) and one `flagged:` line per scenario file left out. |

`--json` prints the server's answer (one line) for `show`, `set`, `validate`, `apply` and `save`. `instance/remote.rs` gained `call_for(method, path, body, total)`, `call` with its own time budget; `call` is `call_for` with 10 s.
