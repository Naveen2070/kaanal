# `crates/kaanal-core/src/instances.rs`

Cloud instance presets (step 8P, RULES 102 b, 110). For users: [../../../web/nodes.md](../../../web/nodes.md#cloud-instances-instance).

## What it does

One embedded table of instance types, `provider/type` in lower case (`aws/m5.large`, `gcp/n2-standard-4`, `azure/standard_d4s_v5`), each with its vCPUs, memory (MiB) and peak network rate (Mbit/s). `instance:` on a node fills `cpu.cores`, `bandwidth` (both ways) and the `memory.limit` the node leaves out: `NodeConfig::cpu()` and `bandwidth()` fall back to it, `NodeConfig::memory()` takes its limit from it. A value the node writes wins.

**Every figure is an estimate** from the providers' public pages as of 2026-11, rounded, and an upper bound: a vCPU counts as a full core, the network figure is the peak rate, burstable types are listed at their full vCPU count (credits are not modelled). A preset is capacity, not speed.

## API

| Item           | Kind   | What it does                                                                        |
| -------------- | ------ | ----------------------------------------------------------------------------------- |
| `Instance`     | struct | `name`, `vcpu`, `memory_mib`, `net_mbps`; `net_bytes_per_s()` and `memory_bytes()`. |
| `INSTANCES`    | const  | The table, sorted by name (binary search).                                          |
| `lookup(name)` | fn     | The instance, case-insensitive and trimmed; `None` when unknown.                    |
| `hint(name)`   | fn     | For the load error: the names of that provider, or how to write one.                |

An unknown name is a load error at `nodes.<name>.instance` (`config/checks.rs`), naming the provider's types.

## Tests

`instances::tests` (sorted and unique, lookup) and `ui_table` (Studio's picker, `ui/src/panels/details/instances.ts`, lists the same names and figures); `tests/config/instance_memory.rs` (what an instance fills, a written value wins, the refusals).
