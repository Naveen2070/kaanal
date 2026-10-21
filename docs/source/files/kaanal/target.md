# `crates/kaanal/src/target.rs` (private)

Where a client command sends its request: the local instance found through `.kaanal/run.json`, or a remote one named by `--endpoint` / `KAANAL_ENDPOINT` with `--token` / `KAANAL_TOKEN` (spec E1-E6).

## What it does

`main` records the two flags once with `set_overrides`; `resolve()` then picks the target for each call. With no endpoint it is the old path: `lifecycle::require_instance` (exit 2 when nothing runs, see [lifecycle.md](lifecycle.md)) and the token from `run.json`. With an endpoint it never touches `run.json`, the instance lock or the stale-state logic; it parses the URL, requires a token and returns a `Target` marked `remote`. The token is never printed, logged or put in an error message.

Precedence per value is flag, then environment variable (an empty one counts as unset), then `run.json`.

## Items

| Item                             | What it does                                                                                                                                                                                                                                                                                                       |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Target`                         | `host` (URL-ready: a name, IPv4 or bracketed IPv6), `port`, `token`, `remote`.                                                                                                                                                                                                                                     |
| `set_overrides(endpoint, token)` | Stores the flags in a `OnceLock`. Env is read when needed.                                                                                                                                                                                                                                                         |
| `endpoint_source()`              | `Some("--endpoint")` or `Some("KAANAL_ENDPOINT")` when an endpoint is set. `up` and `up -d` call it through `no_endpoint` in [main.md](main.md) and exit 64 naming the source (E6).                                                                                                                                |
| `resolve()`                      | The target of a client call. Usage errors (64): a bad endpoint (`bad --endpoint: ...`), an endpoint with no token (`set KAANAL_TOKEN`), and the `--token` **flag** with no endpoint. An exported `KAANAL_TOKEN` alone is ignored (E2). Prints the cleartext warning once per process for a non-loopback host (E4). |
| `Target::base()`, `url(path)`    | `http://HOST:PORT` and a control path on it, as messages show them (never with the token).                                                                                                                                                                                                                         |
| `Target::host_header()`          | Always `localhost:<port>`, which the server's Host guard accepts (R5.3) even when the connection goes to another address (E3).                                                                                                                                                                                     |
| `Target::auth()`                 | The `Bearer` header value.                                                                                                                                                                                                                                                                                         |
| `Target::unreachable(why)`       | A request that got no answer. Remote: exit 1 `cannot reach http://HOST:PORT: why` (exit 2 means "no local instance", which a remote cannot know, E5). Local: the lock decides through `lifecycle::transport_failure`.                                                                                              |
| `Target::refused(status, msg)`   | Remote only: 401 is exit 1 `token rejected`; 403 `bad Host header` is exit 1 with a hint that a port mapping must keep the port number. `None` for anything else, so the usual error path runs.                                                                                                                    |
| `parse_endpoint(s)`              | Accepts `http://HOST:PORT` with one optional trailing `/`; refuses `https://` ("the control API is plain http"), a missing scheme or port, a path, query or fragment, userinfo, port 0, a non-digit port and a bad IPv6. The message never echoes the input, which could hold a password.                          |
| `is_loopback(host)`              | `localhost` (any case), `127.0.0.0/8` or a bracketed `[::1]`; everything else makes the token cross the network and earns the warning (E4).                                                                                                                                                                        |

## Unit tests in this file

`parse_endpoint_accepts_the_forms_people_paste`, `parse_endpoint_rejects_everything_else`, `parse_endpoint_never_echoes_a_password`, `is_loopback_table`, `remote_failures_never_carry_the_token`. End-to-end: [../../tests.md](../../tests.md#json-remote-mode-and-completions).
