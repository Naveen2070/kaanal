# Auth: login and JWKS

## What you get

An `auth` node serves two routes:

- `POST /login` returns a signed RS256 JWT for one of your test users.
- `GET /.well-known/jwks.json` serves the public key that verifies those tokens.

Both behave like any other endpoint: they cost modelled time, obey chaos, show up in the trace header and the request log, honour `error_map`, and work as simulator streams. A config with only an `auth` node is enough for a frontend that has its own login form.

## Config

```yaml
nodes:
  login:
    type: auth
    proc: 5ms
    token_ttl: 2h # whole seconds, 1s to 1 day, default 1h
    claims: { aud: my-app } # extra claims every user gets
    users:
      - username: alice
        password: secret
        roles: [admin]
        scopes: [read, write]
        claims: { email: a@x.dev } # overrides the node's claims
```

Rules, all checked when the file loads (errors carry `line:col`):

- At most one `auth` node.
- Claim names `sub`, `iss`, `exp`, `iat`, `jti`, `roles` and `scope` are reserved and cannot be set.
- A claim value is a string, a number or a boolean.
- At most 32 claims (per node and per user), 32 roles and 32 scopes per user, and 1000 users. Names, roles, scopes, usernames and passwords are at most 256 bytes. A username cannot be empty.
- Scopes are OAuth scope tokens: printable ASCII without spaces, `"` or `\`.
- `token_ttl` is whole seconds from 1s to 1 day.
- A password must be text. A number such as `4711`, or `true` or `null`, is rejected with `must be text` at that line and column (the value is never printed): put it in quotes (`"4711"`). Text with leading zeros such as `007` is kept as it is.

An auth node with no users loads with a warning: every login would answer 401.

## `POST /login`

Send `application/json` or `application/x-www-form-urlencoded` with the fields `username` and `password`. Other fields (such as `grant_type`) are ignored. The body can be at most 16 KiB.

| Status | Body                                                                      | When                                                                                                                                                             |
| ------ | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 200    | `{ "access_token", "token_type": "Bearer", "expires_in" }` (plus `scope`) | Good credentials. `scope` appears only when the user has scopes.                                                                                                 |
| 400    | `{ "error": "invalid_request" }`                                          | A duplicate, missing or non-string `username` or `password`, or unparsable. (A JSON array body such as `["alice", "pw"]` is read as the two fields and logs in.) |
| 401    | `{ "error": "invalid_credentials" }`                                      | Wrong password or unknown user, alike, **after the full chain latency**.                                                                                         |
| 413    | `{ "error": "request_too_large" }`                                        | Body over 16 KiB.                                                                                                                                                |
| 415    | `{ "error": "unsupported_media_type" }`                                   | Any other `Content-Type`, or none.                                                                                                                               |

- It is 401, not OAuth's 400 `invalid_grant`: the mock follows the product's login contract.
- Every response of this route has `Cache-Control: no-store` and `Pragma: no-cache`.
- **The `text/plain` pitfall.** `fetch(url, { method: "POST", body: JSON.stringify(x) })` without a `Content-Type` header sends `text/plain` and gets 415. Add `headers: { "Content-Type": "application/json" }`.
- Other methods get 405.
- When several things are wrong, a chaos failure wins (503 and friends, through `error_map`), then 413, 415 and 400, then 401.

## The token

RS256, with the key id in the header (`kid`). Claims are written in this order, and later ones win:

1. the node's `claims`,
2. the user's `claims`,
3. the standard claims: `sub` (the username), `iss`, `iat`, `exp`, `jti`, `roles`, and `scope` (space-separated, only when the user has scopes).

Tokens differ on every login and every run: they carry the wall-clock time, a random `jti`, and a key made per project.

## JWKS

`GET /.well-known/jwks.json` returns one RSA key whose `kid` is its RFC 7638 thumbprint. A JWT library verifies tokens with it. With `jose` in JavaScript:

```js
import { createRemoteJWKSet, jwtVerify } from "jose";

const jwks = createRemoteJWKSet(new URL("http://localhost:8080/.well-known/jwks.json"));
const { payload } = await jwtVerify(token, jwks, { issuer: "http://localhost:8080" });
```

`HEAD` on the JWKS answers 200 with no body, through the same chain.

## Two ways to model auth latency

- **With an edge `gateway -> auth`**, every request pays an `auth_request` check at the auth node. A login runs inside that call, so the auth node is charged once even if the edge says `calls: 2`.
- **Without the edge**, ordinary requests do not visit auth (the service validates tokens locally). A login still goes gateway, then auth, with no network hop between them.

A login also calls the auth node's own dependencies (a user store database, a session cache, an email API) in the default order. The JWKS costs the auth node's `proc` only.

An `endpoints:` entry for `"POST /login"` (or the JWKS key) can set `chain`, `error_map` or `override`; an `override` replaces the token response. A `HEAD /.well-known/jwks.json` entry is served by the auth node like the JWKS route itself, with or without a gateway, but it does not inherit a custom `chain:` of the `GET` entry: give it its own. If your OpenAPI spec defines its own `POST /login`, the spec route wins with a warning; the JWKS route stays.

## Chaos

`kaanal chaos set login --down` makes login and JWKS answer 503 (through `error_map` if set). `--latency 300ms` slows both. A wrong password is never a chaos failure.

## The signing key

- One RSA 2048 key per project, created on the first `kaanal up` (it prints `Generating signing key...`) and reused after that.
- It is stored outside the project, in your per-user local config folder: `%LOCALAPPDATA%\kaanal\keys` on Windows, `~/.config/kaanal/keys` on Linux, `~/Library/Application Support/kaanal/keys` on macOS. Only you can read it.
- `KAANAL_CONFIG_DIR` changes the base folder (for containers and CI). A shared, writable `XDG_CONFIG_HOME` is not supported.
- The key belongs to the folder of `kaanal.yaml`. **Moving or renaming the project creates a new key**: old tokens stop verifying, and you can delete the old file by hand.
- A corrupt key file stops `up` and names the file. Delete it to get a new key.
- On Linux and macOS a key file that others can read is refused (`chmod 600` fixes it).
- On Windows, `up` also removes explicit grants to Everyone, Users and Authenticated Users from the keys folder and the files in it.
- On a filesystem without hard links the key is created directly instead.
- A crash can leave a `*.tmp` file in the keys folder. It is safe to delete.

`kaanal validate` never creates or reads keys.

## `iss` and the port

`iss` is `http://localhost:<port>` of the running instance. If the configured port is busy, Kaanal moves up to 20 ports, so `iss` can change between runs. Pin `--port` when a client checks the issuer.

## Security notes

- The accounts are synthetic and passwords are plain text in `kaanal.yaml`. Never use real ones.
- Mock routes send `Access-Control-Allow-Origin: *` without credentials, so a `fetch` with `credentials: 'include'` cannot read the answer. Use the default `credentials`.
- Any web page you open can POST to a local mock. That is acceptable here: the accounts are synthetic and nothing verifies these tokens.
- RS256 here uses a pure-software RSA implementation with a known timing side channel (RUSTSEC-2023-0071). It does not matter for a local mock.
- Passwords never appear in logs, the trace header, events or the control API.

## What is not modelled

- Token verification by mocked services (a missing or invalid token answering 401, a wrong role answering 403).
- Refresh tokens and `/userinfo`.
- OIDC discovery and flows (authorization code, PKCE, client credentials).
- Key rotation and ES256.
- `aud` enforcement: a node claim `aud` is just copied into the token.
