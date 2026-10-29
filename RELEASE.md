Tag: v0.0.1

# Kaanal v0.0.1 (pre-release)

The first pre-release of Kaanal, a single binary that pretends to be your backend: OpenAPI 3.0 mocks with generated
data, simulated service and database timing, and live chaos.

This is an early build. Expect breaking changes between `0.0.x` releases.

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
