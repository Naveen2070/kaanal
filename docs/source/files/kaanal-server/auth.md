# `crates/kaanal-server/src/auth.rs`

An auth node's signing key, JWKS and tokens.

## What it does

`AuthKey` holds a 2048-bit RSA key. `Issuer` turns the auth node's users, claims and token lifetime into signed RS256 tokens. Key files are the binary's job (`kaanal` keeps them outside the project); this module only generates, parses and signs. Nothing here prints a password or the private key.

RS256 is hand-rolled over the `rsa` crate. A token is `base64url(header) . base64url(claims) . base64url(signature)`, no padding. The header is `{"alg":"RS256","typ":"JWT","kid":…}`; the signature is PKCS#1 v1.5 over SHA-256 of the first two parts. `RUSTSEC-2023-0071` (an RSA timing side channel) does not matter for a local mock that signs synthetic tokens (R1.16).

## Items

| Item                                  | Kind   | What it does                                                                                                                                                                           |
| ------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `AuthKey`                             | struct | Private key, `kid` and public JWK. `Clone`. `Debug` shows only the `kid`.                                                                                                              |
| `AuthKey::generate()`                 | fn     | A new 2048-bit key from OS entropy.                                                                                                                                                    |
| `AuthKey::from_pem(pem)` / `to_pem()` | fn     | PKCS#8 PEM in and out (LF line endings). A parse error is a fixed message (it never contains the key text and does not repeat the `rsa` crate's text, which can name the wrong thing). |
| `AuthKey::kid()`                      | fn     | The RFC 7638 thumbprint of the public key.                                                                                                                                             |
| `AuthKey::jwks()`                     | fn     | `{"keys":[{kty, use, alg, kid, n, e}]}`: one RS256 signing key.                                                                                                                        |
| `thumbprint(n, e)`                    | fn     | SHA-256 of `{"e":…,"kty":"RSA","n":…}` (sorted, no whitespace), base64url. Pinned by the RFC 7638 section 3.1 vector.                                                                  |
| `Issuer`                              | struct | Key, users, node claims, token lifetime and `iss` (crate-private).                                                                                                                     |
| `Issuer::from_config(cfg, key, port)` | fn     | `None` without an auth node. With one and no key it is an error: the server **fails closed** (X5). `iss` is `http://localhost:<bound port>`, never `Host`.                             |
| `Issuer::login(username, password)`   | fn     | The 200 body (`access_token`, `token_type`, `expires_in`, and `scope` with scopes), or `None` (a 401) for an unknown user or a wrong password alike.                                   |

## Login requests

| Item                        | What it does                                                                                                                                                                                                                                                                                                                                             |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Credentials`               | `username` and `password`, read with serde from JSON or a form: duplicate, missing and non-string fields are rejected, extras (`grant_type`) ignored. `Debug` redacts the password. Quirk: serde also reads a JSON array, so `["alice","pw"]` logs in.                                                                                                   |
| `Reject`                    | `(status, error code)` of a request rejected before the credentials are checked. Never a parser message (those can quote the password).                                                                                                                                                                                                                  |
| `read_login(req)`           | Runs before the chain. `Content-Type` (case-insensitive, parameters ignored) must be `application/json` or `application/x-www-form-urlencoded`, else 415, decided from the header alone. Body over 16 KiB (any read error) is 413; anything serde rejects is 400. A browser `fetch` with `JSON.stringify` and no header sends `text/plain` and gets 415. |
| `respond(r, issuer, login)` | The answer after the chain succeeded: JWKS 200; login 200 with the token body, 401 `invalid_credentials`, or the `Reject` (400 `invalid_request`, 413 `request_too_large`, 415 `unsupported_media_type`).                                                                                                                                                |

## Claims

The node's `claims`, then the user's `claims` (which win over the node's), then the standard claims `sub`, `iss`, `iat`, `exp`, `jti`, `roles` and `scope` (only when the user has scopes, space-joined). The standard claims are written last, so they always win: reserved names are rejected at load, and this also holds for a `Config` built in code without that check. `iat` and `exp` are whole seconds; `exp = iat + token_ttl` (default 1 h). `jti` is 128 random bits as 32 lowercase hex digits. Tokens are not reproducible (wall clock, random `jti`).

## Dependencies

`rsa` 0.9 (with `sha2`), `sha2` 0.10 (0.11 uses a `digest` that `rsa` 0.9 cannot sign with), `base64`. `jsonwebtoken` is a **dev-dependency only**: tests verify our tokens with it as an independent implementation. The dev profile builds `rsa` and `num-bigint-dig` with `opt-level = 3`, because an unoptimised 2048-bit key generation takes seconds.

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcauthrs-unit--key-jwks-and-tokens) — thumbprint vector, JWKS shape, PEM round-trip, independent verification, claim precedence, rejected logins, fail closed.
