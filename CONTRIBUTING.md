# Contributing to Kaanal

Thanks for helping. Bug reports, docs fixes and focused PRs are all welcome.

## Before you start

- **Bug or idea?** Open an issue first for anything bigger than a small fix, so we agree on the approach before you write code.
- **Security problem?** Do not open an issue. See [SECURITY.md](SECURITY.md).
- **Scope.** Kaanal models backend _behaviour_ (latency, capacity, failure, data). It does not emulate wire protocols or run SQL. Requests for Kafka, gRPC, plugins or containers will usually be declined for now.

## Set up

You need **Rust 1.100+**, and **Node.js + pnpm** for Studio (`ui/`) and the docs site (`site/`).

```sh
git clone https://github.com/Naveen2070/kaanal.git && cd kaanal
(cd ui && pnpm install && pnpm build)   # Studio, embedded into the binary
git config core.hooksPath .githooks     # run the checks before each commit
cargo build --workspace
```

Try your build: `cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml`.

## Check your change

CI runs all of these. The pre-commit hook runs the ones your staged files can affect.

| Changed             | Run                                                                                                          |
| ------------------- | ------------------------------------------------------------------------------------------------------------ |
| Rust                | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` |
| Studio (`ui/`)      | `cd ui && vp check`, then the UI tests                                                                       |
| Markdown            | `dprint check`                                                                                               |
| A lock file         | `node scripts/third-party-licenses.mjs --check` (regenerate the file without `--check` if it fails)          |
| Any source file     | `bash scripts/check-file-size.sh` (800 lines per source file, 1000 per test file)                            |
| Docs site (`site/`) | `cd site && pnpm build`                                                                                      |

The pre-commit hook runs only the tests a change can reach (the changed test files, the unit tests of a changed module, or that crate’s suite). `KAANAL_HOOK=deps git commit ...` also tests the crates that depend on it; `KAANAL_HOOK_FULL=1 git commit ...` runs everything. CI always runs everything.

## Code expectations

- **Match the style.** Plain code, precise errors (`file:line:col`), rustdoc on every item, comments that say _why_. Read the [coding guide](docs/coding-guide/index.md) first.
- **Keep the layering.** `kaanal-core` is pure (no async, IO or clock); only `kaanal-server` speaks HTTP; the CLI and Studio are thin clients of `/__kaanal/*`. The [architecture](ARCHITECTURE.md) explains why.
- **Stay deterministic.** The same `seed` must give the same output. No std hashers, wall-clock time or environment state in output paths.
- **Add a test.** A bug fix gets a test that fails without it. A model change gets a theory test against a published result where one exists.
- **Recipes.** [docs/source/how-to.md](docs/source/how-to.md) covers adding a CLI command, API route, node type or step kind.

## Commits and pull requests

- Work on a branch, not `main`. Open the PR against `dev`.
- **Sign off every commit** (`git commit -s`). This is the [Developer Certificate of Origin](https://developercertificate.org/): you wrote it or have the right to submit it under Apache-2.0.
- Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `perf:`, `chore:`, with an optional scope (`feat(server): ...`). Add a short bullet list of what changed.
- Keep a PR to one topic, and say how you checked it.

## License

By contributing you agree your work is licensed under [Apache-2.0](LICENSE).
