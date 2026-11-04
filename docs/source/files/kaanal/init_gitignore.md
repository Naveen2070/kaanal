# `crates/kaanal/src/commands/init/gitignore.rs` (private)

`ignore_state_dir` and its helper, moved out of `init/mod.rs` unchanged (re-exported there as `init::ignore_state_dir`, which `kaanal demo --eject` also uses).

| Item                | What it does                                                                                                                                                                                                                                                                                  |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ignore_state_dir`  | Reads `.gitignore` only to find the line (`.kaanal`, `.kaanal/`, `/.kaanal`, `/.kaanal/`, `.kaanal/*`, `.kaanal/**`), then appends with `OpenOptions::append` in the file's own line ending. Unreadable or non-UTF-8 files are never touched: `Err(reason)`. `Ok(false)` when already listed. |
| `ignores_state_dir` | One line counts as already ignoring the state directory.                                                                                                                                                                                                                                      |
