Tag: v0.0.1

# Kaanal v0.0.1 (pre-release)

The first pre-release of Kaanal, a single binary that pretends to be your backend: OpenAPI 3.0 mocks with generated
data, simulated service and database timing, and live chaos.

This is an early build. Expect breaking changes between `0.0.x` releases.

## How a streamed response is costed

A read of a storage node now streams: its bytes cross a wire or card with a `bandwidth` while the store is still
reading, so the call takes the first byte plus the size over the slowest stage, not the stages added. This only moves
figures with a `bandwidth`. `stream: false` on the edge or the `call:` step restores the old sum; `stream: true` turns
it on for any callee.

## How a new connection is costed

An edge with a `net` (its round trip) opens a new connection for 1 call in `keepalive` (default 1000); that call
pays the TCP handshake (1 round trip), TLS if `tls` is set (1 for 1.3, 2 for 1.2), and TCP slow start for a body
that does not fit the first 14,600 B window. Reused connections pay nothing. An edge without `net` pays nothing.
Existing files with a `net` therefore move by about 1 call in 1000; set `keepalive: 1000000` to leave it out.

## How body sizes are costed

Sizes only matter when a topology has a `bandwidth` (on an edge or a node), a `per_kb`, or a storage `throughput`.
Then:

- **Headers count.** Every wire transfer and every HTTP node's `per_kb` carries 600 B on a request and 400 B on a
  response, an estimate of real HTTP headers. Write `header_bytes: 0` to leave them out, or a size, or
  `{ request, response }`, to change them.
- **Direction follows the operation.** A write (`PUT`, an insert) counts the request body, a read (`GET`) and a scan
  count the response body, and a service counts both. This is how HTTP and object stores move data; there is no
  switch for it.
- **A call can carry its own sizes.** `sends:` and `returns:` on an edge or a `call:` step say what that call
  carries (a 5 MB upload that becomes a 200 KB call to storage); without them the incoming size is passed on.
- **Transfers share a pipe fairly** instead of queueing, so a big upload no longer blocks a small call.

A file that has a `bandwidth`, a `per_kb` or a `throughput` therefore reports different times from an earlier
build: the headers add to every transfer, and a write no longer pays for the reply it never sends.
`kaanal validate` also warns when an OpenAPI route takes or returns a file, because the spec does not give its size.

## Install

Download the archive for your system, check it against `SHA256SUMS`, unpack it and run `kaanal --help`.

| System  | Archive                                         |
| ------- | ----------------------------------------------- |
| Linux   | `kaanal-v0.0.1-x86_64-unknown-linux-gnu.tar.gz` |
| macOS   | `kaanal-v0.0.1-aarch64-apple-darwin.tar.gz`     |
| Windows | `kaanal-v0.0.1-x86_64-pc-windows-msvc.zip`      |

Try it with `kaanal demo`.

<!--
Maintainer notes (an HTML comment, so it does not show in the published release):
- The UI build is part of a release: release.yml builds `ui/` and fails before `cargo build` if
  `crates/kaanal-server/ui-dist/{index.html,app.js,app.css}` is missing. Nothing to do by hand.
- [ ] crates.io step, when there is one: `cargo package` drops git-ignored files, so `ui-dist/` must be built
  and included in the package first (Cargo.toml has no `include` list on purpose). Until then, do not publish.
-->
