# `crates/kaanal-core/src/cost.rs`

The cost estimate as arithmetic (RULES 121, 123 to 126, 131). For users: [../../../web/nodes.md](../../../web/nodes.md#cloud-instances-instance) and [simulation.md](../../../web/simulation.md).

## What it does

`estimate(&Config, &visits) -> CostReport` prices every node of a loaded config: copies (`replicas`) times the price an hour of its instance, summed. Money is whole micro-USD (1,000,000 = 1 dollar) so sums are exact. The server calls it when a run finishes (`sim/run.rs`) and puts the report on the run record; Studio's cost card does the same sums on the page (`ui/src/panels/cost/cost.ts`), and `tests/cost.rs` and `cost.test.ts` hold the two to the same published figures.

Where a price comes from, in order: the node's hosting (`serverless`, `external` and a managed gateway have none: the node says why), a missing instance (`no instance type`, or `pick a managed ... class`), your own instance (`cost_per_hour`), a managed database class priced per engine (`prices::managed_price_micro_usd_hr`), a managed cache class priced per flavor (`managed_cache_price_micro_usd_hr`), else the VM price (`price_micro_usd_hr`), each in the region of its cloud: the file's `region:` entry, else `prices::DEFAULT_REGIONS`. `multi_az: true` on a managed non-Aurora database doubles the unit price (`MULTI_AZ_FACTOR`, AWS bills the standby as a second instance). An object store is one bill: its data kept over a month (`stored`, `HOURS_PER_MONTH` = 730) plus its requests at the node's `visits` a second, a `write_share` of them at the write price; its own `location:` beats the region of its cloud.

## API

| Item                     | What it is                                                                                                                                                                                               |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `estimate(cfg, visits)`  | The report; `visits` is each node's visits a second (a run's `visits_per_s`), empty to price data alone.                                                                                                 |
| `regions_of(cfg)`        | The region each cloud is priced in.                                                                                                                                                                      |
| `NodeCost`, `CostReport` | The answer (serializable): per node `unit_micro_usd_hr`, `total_micro_usd_hr`, `copies`, `instance`, `why`, `note`; the report adds the total, the priced and unpriced counts, the regions and `egress`. |

`estimate_with_egress(cfg, visits, &flows)` adds what a run sends out to the internet (RULES 131): `egress_flows(cfg, topo, endpoints, streams)` turns each stream's `rate x resp_bytes` into bytes a second by the cloud of its **entry node** (the prefix of its `instance:`; a node that names no cloud is priced at AWS's rate and says so), and each cloud's line is the GB a month less the free allowance (`prices::egress_price`) at the first-tier price, over 730 hours. The server passes the run's streams in `sim/run.rs`; the report gets `egress` (one `EgressCost` per cloud: bytes a second, fee an hour, note) and the total includes it; the CLI prints it under the cost line. A `hosting: serverless` service or monolith is priced as AWS Lambda from its visits (requests plus GB-seconds of its mean `proc` at its `memory:`, 128 MB when none), a `hosting: managed` gateway as an API Gateway REST API (per request); both need the run's `visits` and say "priced by use, after a run" without them, and `NodeCost.note` says what was assumed. The Azure SQL engine is chosen by the class's cloud (`azure/db.*`).

Not in it: egress tiers beyond the first and CDN routes, reserved and spot prices, DynamoDB, ALB and Aurora Serverless usage, Lambda time spent waiting on downstream calls, storage and I/O charges of a managed database, retrieval fees of a store beyond `retrieved`. A node in one of those says so in `why`, or is simply not covered (see RULES 126, 131). Egress is on the run record and the CLI, not in Studio's live cost card.

## Tests

[`tests/cost.rs`](../../../../crates/kaanal-core/tests/cost.rs): a system priced against AWS's published t3.micro ($0.0104), db.t3.micro ($0.018) and Multi-AZ, the file's region and a cache's engine, an object store's data and requests, every `why`, your own instance, and the JSON.
