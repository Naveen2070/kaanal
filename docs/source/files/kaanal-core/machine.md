# `crates/kaanal-core/src/config/machine.rs`

Your own instance types (`instances:` at the top of `kaanal.yaml`, RULES 113). For users: [nodes guide](https://kaanal.naveenr.in/docs/reference/nodes/).

## What it does

`CustomInstance { cores, memory, bandwidth }` is an entry of `Config::instances`. A node names one with `instance: name`; names have no `/` (the cloud table is `provider/type`), so the two never collide. `check` runs while the file loads: a usable name (1 to 64 of letters, digits, `.`, `_`, `-`), cores valid like `cpu.cores`, memory and bandwidth at least 1, and every node's `instance:` naming an entry or a table type (the error lists yours). `apply` then **writes** each custom instance into the nodes that name it (`NodeConfig::fill_machine`: `cpu`, `bandwidth` and the `limit` of a `memory:` the node sets, only where the node left them out), so everything after sees plain `cpu`, `bandwidth` and `memory` and needs no second code path. A table type is not copied: the accessors read the table.

## Tests

`tests/config/instance_memory.rs`: it fills like a table type, the node's own values win, a partial instance fills only what it has, several nodes share one, the refusals with their text.
