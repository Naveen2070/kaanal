# `crates/kaanal-server/src/fingerprint.rs`

The system's design fingerprint: 12 hex digits a sweep report prints so a reader can tell whether two sweeps ran on the same design. Built from the blocks in [../kaanal-core/fingerprint.md](../kaanal-core/fingerprint.md).

## What it does

`Generation::build` calls `of_system(cfg, base_dir)` right after the loaders ran and stores the result in `Generation.fingerprint`. It is `combine` over one `(label, digest)` part per input:

| Input                                    | Label                            | How it is found                                                                        |
| ---------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------- |
| The config                               | `config`                         | `Config.digest`, the canonical digest of the exact text `Config::from_yaml` parsed     |
| Each node's `spec:` file                 | its path relative to the project | `NodeConfig::spec`, resolved by `paths::resolve` with `Trust::Owner` (as `load_specs`) |
| The `endpoints:` file, when it is one    | its path relative to the project | `paths::resolve` with `Trust::Owner` (as `compile_endpoints`)                          |
| `scenarios/*.yaml`                       | `scenarios/<name>.yaml`          | `scenarios::list` over `generation::scenarios_dir` (as `load_scenarios`)               |
| `locales/*.yaml`, `data:` or `data.yaml` | its path relative to the project | `kaanal_data::DataSet::files` (the rules of `DataSet::load`)                           |

Labels use forward slashes. Each file is read once (two nodes naming one spec count it once) with `paths::read_text` and hashed with `canonical_digest`, so comments, whitespace and key order do not change the fingerprint. A missing optional input (no `scenarios/`, `locales/`, `data.yaml` or endpoints file) adds nothing, so an empty `scenarios/` folder gives the same value as none. The `site:` folder is not an input.

**Never fails.** A file that cannot be read now, though the build read it a moment ago, keeps its label with a fixed all-zero digest; a listing that fails now (`scenarios/` or `locales/`) adds its folder label with that digest. Either way the value differs from that of a project without the input: a false "different" is harmless, a false "same" is not.

**Known and accepted.** The files are read again here, right after the loaders and from the same `base_dir`; only the config's digest is of the exact bytes parsed. An edit landing between the loader's read and this one would be counted in the fingerprint but not in the build.

## Items

| Item                       | Kind            | What it does                                                                  |
| -------------------------- | --------------- | ----------------------------------------------------------------------------- |
| `of_system(cfg, base_dir)` | fn (crate)      | The fingerprint, 12 lower-case hex digits                                     |
| `parts(cfg, base_dir)`     | fn (private)    | The labelled digests above, `config` first                                    |
| `label(base_dir, file)`    | fn (private)    | `file` relative to `base_dir`, forward slashes; as is when it is not under it |
| `UNREADABLE`               | const (private) | `[0; 32]`, the digest of an input that cannot be read or listed now           |

## Tests

Unit tests in the file, each on its own temp project: comments and key order keep the value; editing the config's `seed:`, the spec, the endpoints file, a scenario, a locale pack or `data.yaml` changes it; a configured `data:` file replaces `data.yaml`; a project without optional inputs is stable (and an empty `scenarios/` adds nothing); a deleted input still gives a value; `Generation::build` carries it. `every_loader_input_is_counted` is the drift guard: it corrupts every file of a project with every kind of input (plus a `site:` folder) and checks that the files whose corruption fails the build are exactly the counted parts.
