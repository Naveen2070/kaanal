# `crates/kaanal/src/keys.rs` (private)

The per-project signing key on disk.

## What it does

IO only: generating, parsing and signing live in `kaanal_server::AuthKey`. `up` calls `signing_key(config)` **only when the config has an auth node**, after the instance lock and before binding the port. `validate` and `endpoints` never touch keys.

## Where the key lives

`<base>/kaanal/keys/<id>.pem`

- `<base>` is `KAANAL_CONFIG_DIR` (an empty value counts as unset), else the per-user local config directory (`dirs::config_local_dir()`: `LocalAppData` on Windows, so the key never roams with the profile). With neither, `up` stops and says to set `KAANAL_CONFIG_DIR`.
- `<id>` is the first 16 hex digits of the SHA-256 of the **canonical directory of `kaanal.yaml`** (`\\?\` stripped on Windows). A project keeps one key whatever the config file is called; moving the project starts a new key and orphans the old file.

## Flow

1. `private_dir` on the keys directory, **on every run** (Unix mode 0700 or an error; Windows ACL replaced by one grant for the current user, a warning if that fails). It also repairs a directory the user widened.
2. Read the file. Found: `parse`. Not found: print `Generating signing key...`, generate, `store`, and use whichever key the file ends up holding.
3. Any other read error stops startup naming the file.

## Items

| Item                    | What it does                                                                                                                                                                                                                                                                                                                                                                                                          |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `signing_key(config)`   | The flow above. Returns an `AuthKey`.                                                                                                                                                                                                                                                                                                                                                                                 |
| `keys_dir(over, local)` | `<base>/kaanal/keys` from the override or the local config dir (pure, so the no-directory error is unit-tested).                                                                                                                                                                                                                                                                                                      |
| `key_id(config)`        | The 16-hex identity of the config file's directory.                                                                                                                                                                                                                                                                                                                                                                   |
| `store(dir, file, pem)` | Writes `pem` to a unique temp file (`<id>.<pid>-<nanos>.tmp`, `create_new`, mode 0600 on Unix, `sync_all`), then `hard_link`s it to `file` and removes the temp. If the name already exists (another `up` won the race), it adopts the winner's key. If hard links are unsupported it creates the final name directly instead (`create_direct`). Returns the PEM `file` holds. The final name never holds half a key. |
| `parse(file, pem)`      | On Unix a file readable by group or others is refused with a `chmod 600` hint. Text that is not a PKCS#8 RSA key is an error naming the file ("delete the file to generate a new key"); the file is **never replaced**.                                                                                                                                                                                               |

## Not handled

- A crash between the temp write and the link leaves a `*.tmp` file in the keys directory. It is ignored and safe to delete.
- A shared, writable `XDG_CONFIG_HOME`.
- Filesystems without hard links (FAT, some network shares): `store` falls back to creating the final name directly (`create_new`, mode 0600, a failed write removes the partial file). That is not atomic against a second `up` reading the file mid-write, which could then stop with "delete the file to generate a new key"; it needs a second `up` of the same project in the same instant on such a filesystem.
- Windows has no group/other mode check: the directory ACL is the protection. `private_dir` removes inherited entries and explicit grants to Everyone, Users and Authenticated Users (folder and files in it); an explicit grant to another named user or group is left alone.

## Tests

[../../tests.md](../../tests.md#crateskaanalsrckeysrs-unit--the-signing-key-file) — unit tests for the directory, the identity, the race and the corrupt file; CLI tests for generate-once-then-reuse, a pre-seeded key, a corrupt file, `validate` never touching keys, the `endpoints` command, and the permission checks.
