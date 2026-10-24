# `crates/kaanal/src/commands/scenario.rs`

`kaanal scenario` and `kaanal endpoints`: list, activate, save and print scenarios, and write missing endpoints. Split out of `main.rs` (R3.8); the overview of the commands is in [main.md](main.md).

## Functions

| Function                                       | What it does                                                                                                                                   |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `scenario(cmd)`                                | Dispatches the `scenario` subcommands to the calls above.                                                                                      |
| `scenario_path(name, tail)`, `active_names(v)` | `/__kaanal/scenarios/<percent-encoded name><tail>`; the `active` names of a stack answer.                                                      |
| `scenario_body(cfg)`                           | The `PUT` body of a parsed file: `{ description?, chaos }`, sending `cfg.chaos` directly (`ChaosEntry` is `Serialize` and skips unset fields). |
| `named(name)`                                  | Checks a name with `valid_name` first, so a reserved one (`active`, `save`, `reset`) gets a message instead of hitting another route.          |
| `save_from(from)`                              | `live` / `effective` / `scenario:NAME` to the API's tagged JSON.                                                                               |
