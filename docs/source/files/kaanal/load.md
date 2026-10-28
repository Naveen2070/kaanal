# `crates/kaanal/src/util/load.rs`

Loading and checking `kaanal.yaml`: capped reads, positioned config errors, the auth user list. Split out of `main.rs` (R3.8); the overview of the commands is in [main.md](main.md).

## Functions

| Function                      | What it does                                                                                                                                                                                                       |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `config_err(path, e)`         | `file:line:col: msg` when the error has a position, else `file: msg`. `validate` and `up` use it with `Located::in_source` for server errors.                                                                      |
| `located_err(config, src, e)` | Prints a server load error against the file it is in: the endpoints file (read again for the line and column) when `Located.file` is set, else `kaanal.yaml`. `validate` and `up` use it.                          |
| `load(path)`                  | Reads + parses config; formats errors as `file:line:col: msg` (R6.1); returns the config's directory for resolving spec files and the source, so later errors can be placed.                                       |
| `blank_loaded(path)`          | What a blank start runs on: the default config (its port and seed), the folder of `path`, and `BLANK_TEXT`. `path` is not read.                                                                                    |
| `auth_users(cfg)`             | The auth node's user count, or `None` without an auth node. `up` prints the `Auth:` line (`POST …/login, JWKS …/.well-known/jwks.json, N users`) after `UI:` from it. `validate` and `endpoints` never touch keys. |
