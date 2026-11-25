# Security Policy

## Supported versions

Kaanal is pre-1.0. Only the latest release gets security fixes.

| Version            | Supported |
| ------------------ | --------- |
| latest 0.x release | yes       |
| older releases     | no        |

## Reporting a vulnerability

Please **do not open a public issue** for a security problem.

Report it privately through GitHub: [Security → Report a vulnerability](https://github.com/Naveen2070/kaanal/security/advisories/new).

Include:

- the Kaanal version (`kaanal --version`) and your OS,
- the steps or a `kaanal.yaml` that reproduce it,
- what you expected and what happened, and the impact you see.

You can expect an acknowledgement within a few days. Kaanal is maintained by one person, so fix times vary; we will keep you updated and credit you in the release notes unless you prefer not to be named.

## Scope

Kaanal is a **local development tool**. It binds `127.0.0.1` by default and is not meant to face the internet.

In scope:

- bypassing the control-plane protections: the loopback `Host` check, the Bearer token on `/__kaanal/*`, the CSP, or the no-CORS rule on control routes,
- leaking the control token (logs, URLs, redirects, proxies, file permissions),
- reading or writing files outside the project and `.kaanal/` directory,
- crashes or unbounded memory or CPU from a crafted `kaanal.yaml` or OpenAPI file (for example recursive `$ref`),
- a flaw in the release archives, npm packages or crates.

Out of scope:

- exposing Kaanal on a public interface on purpose (`--host` set to a non-loopback address) and then being reached from the network,
- anything that needs local access to the machine as the same user (the token file is user-private by design),
- the simulator's numbers being a prediction (it is a relative what-if).

## Design notes

How the guard works is in [ARCHITECTURE.md](ARCHITECTURE.md) and the [architecture guide](docs/source/architecture.md#7-control-plane-security--event-bus).
