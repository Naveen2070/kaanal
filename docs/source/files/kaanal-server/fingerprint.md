# `crates/kaanal-server/src/fingerprint.rs`

The system's design fingerprint: 12 hex digits a sweep report prints so a reader can tell whether two sweeps ran on the same design files. It covers the design files only; the chaos a sweep ran under is recorded beside it on the sweep record (`live_chaos`, RULES 140). Built from the blocks in [../kaanal-core/fingerprint.md](../kaanal-core/fingerprint.md).

## What it does

`Generation::build` (through `build_counted`, see [generation.md](generation.md)) makes an `Inputs` and hands it to the loaders; each loader adds every input as it reads it, and the build stores `inputs.fingerprint(cfg.digest)` in `Generation.fingerprint`. It is `combine` over one `(label, digest)` part per input:

| Input                                    | Label                            | Handed over by                                                                                                                   |
| ---------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| The config                               | `config`                         | `Config.digest`, the canonical digest of the exact text `Config::from_yaml` parsed                                               |
| Each node's `spec:` file                 | its path relative to the project | `load_specs`: `Inputs::parsed` with the document `ApiModel::from_openapi_doc` parsed (`value_digest`, so it is not parsed twice) |
| The `endpoints:` file, when it is one    | its path relative to the project | `compile_endpoints`: `Inputs::text` with the text it parsed                                                                      |
| `scenarios/*.yaml`                       | `scenarios/<name>.yaml`          | `load_scenarios`: the text `scenarios::discover` read (a file a draft build leaves out, `flagged`, still counts)                 |
| `locales/*.yaml`, `data:` or `data.yaml` | its path relative to the project | `load_data`: the sources `kaanal_data::DataSet::load_with_sources` read                                                          |

Labels use forward slashes; a spec or endpoints path is labelled by `rel_label` (resolved by the owner's rule, so a draft build, `Trust::Api`, labels a file as a boot does). A file read twice (two nodes naming one spec) counts once. Every digest is the canonical one (`canonical_digest`, or `value_digest` of the parsed document), so comments, whitespace and key order do not change the fingerprint. A missing optional input adds nothing, so an empty `scenarios/` folder gives the same value as none. The `site:` folder is not an input.

**No second reading.** Until 2026-11-21 the fingerprint listed and read the files again after the loaders (an accepted race: a file saved in between was counted in its newer form) and parsed each again. Measured then (release, 21 builds, median): on `internal/dummy/db-choke` the fingerprint was 0.49 ms of a 1.10 ms build; on 50 specs of 1 MiB, 4.2 s of a 6.6 s build; on one 16 MiB spec, 2.0 s of 2.8 s. Now nothing is read twice and a spec is not parsed twice: the fingerprint's own work is 0.06 ms on the full test fixture, 111 ms on 50 MiB of specs (about 5% of a 2.2 s build), 36 ms on 16 MiB (about 5% of 0.7 s); with `value_digest` streaming into the hash (no 16 MiB buffer), 92 ms (p95 121) of a 2.1 s build and 29 ms (p95 44) of a 0.7 s build, about 4%. What is left is linear in the bytes (one JSON serialisation and one SHA-256 per input); the small inputs (endpoints, scenarios, packs, the dictionary: 1 MiB caps) are still parsed once more by `canonical_digest`, as their loaders deserialize straight to typed values.

## Items

| Item                       | Kind           | What it does                                                                  |
| -------------------------- | -------------- | ----------------------------------------------------------------------------- |
| `Inputs`                   | struct (crate) | Label to digest of every input one build read                                 |
| `Inputs::text`             | fn (crate)     | Adds an input by its text (`canonical_digest`)                                |
| `Inputs::parsed`           | fn (crate)     | Adds an input by the document the loader parsed (`value_digest`)              |
| `Inputs::fingerprint`      | fn (crate)     | `combine` over `config` and every input: 12 lower-case hex digits             |
| `Inputs::labels`           | fn (test)      | The labels counted, `config` included, sorted                                 |
| `rel_label(base_dir, rel)` | fn (crate)     | The label of a path as the config names it, resolved by the owner's rule      |
| `label(base_dir, file)`    | fn (crate)     | `file` relative to `base_dir`, forward slashes; as is when it is not under it |

## Tests

Unit tests in the file, each on its own temp project: comments and key order keep the value; editing the config's `seed:`, the spec, the endpoints file, a scenario, a locale pack or `data.yaml` changes it; a configured `data:` file replaces `data.yaml`; a project without optional inputs is stable (and an empty `scenarios/` adds nothing); `the_build_fingerprint_equals_a_fresh_reading_of_its_files` compares the build's value and labels with an oracle that lists and reads the files again (the old method) on a quiet project; a draft build (`Trust::Api`) labels and fingerprints as a boot does. `every_loader_input_is_counted` is the drift guard: it corrupts every file of a project with every kind of input (plus a `site:` folder) and checks that the files whose corruption fails the build are exactly the labels the build counted.
