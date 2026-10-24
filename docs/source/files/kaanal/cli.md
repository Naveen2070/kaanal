# `crates/kaanal/src/cli.rs`

The command-line shape: clap types for every subcommand, the `Cli` parse fix-ups, `schema` and `completions`. Split out of `main.rs` (R3.8); the overview of the commands is in [main.md](main.md).

## Functions

| Function             | What it does                                                   |
| -------------------- | -------------------------------------------------------------- |
| `completions(shell)` | Writes the script for `shell` from `Cli::command()` to stdout. |
