# Simulation

Kaanal can run your system model on a virtual clock: no real requests, no waiting. You give it a [traffic profile](traffic.md) and it tells you how the system would behave: latency percentiles, which node is the bottleneck, where queues grow, how many requests fail and why. The result is a **relative what-if, not a prediction**: it is the same cost model the mocks use, fed with a smooth Poisson stream, so use it to compare designs and find the weak node, not to promise a number.

## Run one

```bash
curl -s -X POST localhost:8080/__kaanal/sim/run \
  -H "Authorization: Bearer $TOKEN" \
  -d '{ "profile": "peak", "duration_s": 60 }'
# 202 {"id":"s1","state":"running"}   Location: /__kaanal/sim/runs/s1
curl -s localhost:8080/__kaanal/sim/runs/s1 -H "Authorization: Bearer $TOKEN"
```

The body:

| Field        | Meaning                                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `profile`    | A profile name from `traffic:`, or a profile written out (`{ "rps": 50, "journeys": [ ... ] }`).                             |
| `duration_s` | Simulated seconds (default: the profile's, else 60; at most 3600).                                                           |
| `warmup_s`   | Simulated seconds before results count (default a tenth of the duration).                                                    |
| `seed`       | Seed of the draws (default the config's `seed:`). The same inputs give the same report.                                      |
| `scenario`   | Run under this [scenario](scenarios.md)'s chaos alone. Without it the run uses the **live chaos** in force when it starts.   |
| `source`     | `applied` (default) runs the system in force; `draft` runs the draft text without Apply (see below). Anything else is `400`. |

A profile name the config does not have is a `404` whose message says what to do: that no profile is defined (add a `traffic:` section or pass `-f`), or the names that are (`name one with -p: peak`); see [traffic](traffic.md).

A run is answered at once with `202`; poll `GET /sim/runs/{id}` until `state` is `done` (or `failed`, with an `error`). `GET /sim/runs` lists the last eight runs without their reports.

## Live and batch

A **batch run** (the rest of this page) simulates a stretch of time at once, seeded: the same inputs give the same figures, so it is the tool for comparing two settings. The **live simulation** (Studio's default view) runs the same engine on the wall clock with generated traffic, so the figures are what the system is doing now, not a finished measurement, and a live run is seeded but not repeatable (timing follows the clock). Both use the same model, so a setting means the same in each: `cpu`, slots, queues, retries, breakers, balancing, `detect` and `from_ms` all apply. Live figures are counts per 250 ms (served, dropped by cause, queue, slots in use, cpu load) and a latency histogram with 1% precision; see the [control API](control-api.md#the-live-simulation-get-__kaanallive-and-its-commands).

## Run on the draft

With `"source": "draft"` the run simulates the text of the draft (`GET /draft`) as it is now, without Apply: nothing is swapped, no counter is reset, mock traffic is untouched. The live chaos is carried onto the draft by name (what the draft no longer has is left out), and a `scenario` resolves against the draft's scenarios. A draft that does not build is `400` with `{ "error", "diagnostic" }` and does not take the run slot. The run record says `"source": "draft"` and a `draft_rev` (the draft's `rev`, which `GET /draft` shows and which counts each change of its text), so a result can be matched to the edit it describes; `generation` is the one in force.

## Load sweep: where does it break?

A single run answers "how does the system behave at this rate". A **load sweep** answers "how much more traffic can it take before it misses a target". It runs your profile at a ladder of **load factors** `k` (multiples of the profile's rate: 0.25x, 0.35x, 0.5x, ... up to `max_factor`, each step x1.414), checks every point against a **limit** (p99 latency and error rate), and then bisects between the last point that passed and the first that failed until they are within 2%. The last passing `k` is the **knee**.

```bash
curl -s -X POST localhost:8080/__kaanal/sim/sweep \
  -H "Authorization: Bearer $TOKEN" \
  -d '{ "profile": "peak", "p99_ms": 500, "error_pct": 1, "max_factor": 8 }'
# 202 {"id":"s2","state":"running"}   Location: /__kaanal/sim/runs/s2
```

| Field        | Meaning                                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `profile`    | A profile name from `traffic:`, or a profile written out, as for a run. Without it, the first profile of `traffic:` (no profiles is `400`). |
| `p99_ms`     | Highest p99 latency that passes, in milliseconds; above 0 (default 1000).                                                                   |
| `error_pct`  | Highest share of failed requests that passes, in percent, 0 to 100 (default 1).                                                             |
| `max_factor` | Top of the ladder, 1 to 64 (default 8). Anything outside is `400`.                                                                          |
| `seed`       | Seed of the draws, the same at every point, so two points differ only in load (default `seed:`).                                            |
| `scenario`   | Sweep under this scenario's chaos alone; without it, the live chaos in force when it starts.                                                |

A sweep always runs the **system in force**: there is no `source` (no draft sweep), and no `duration_s` or `warmup_s`, because each point sizes its own window to about 20 000 reported requests (never shorter than ten times the slowest node's mean processing time). A body with any of those fields is `400`.

A sweep is a run record with `"kind": "sweep"` (a plain run says `"run"`), under the same single slot: a sweep while a run runs, or the other way round, is `409` with the running id. Poll `GET /sim/runs/{id}`: `sweep_points` grows as each point finishes, and when `state` is `done`, `sweep` holds the finding (and `sweep_points` is emptied: `sweep.points` has every point):

- **`verdict`**: `{ "kind": "knee", "k", "req_s" }`, the last passing point; `broken_at_lowest`, already over the limit at 0.25x; or `holds_to`, no failure up to the last point that ran.
- **`seed`**, **`max_factor`**, **`fingerprint`**, **`inline_profile`** and **`live_chaos`**: what was swept, so a report can name it. `seed` is the body's, else the config's; `max_factor` is the checked top of the ladder; `fingerprint` is 12 hex digits over the design files the build read; `inline_profile` is the posted profile when it was written out (absent for a named profile); `live_chaos` is the chaos in force when a sweep without `scenario` started: `{ "scenarios": [active scenarios, bottom to top], "entries": N, "digest": "12 hex digits" }` (the live entries of `PUT /chaos` by name; the order they were set in and the config's node order do not change the digest). A sweep with `scenario` has no `live_chaos`: the scenario named is its chaos. Every sweep also has `chaos_digest` (the compiled chaos, see the design fingerprint below) and, when a scenario in force was defined through the control API (`PUT /scenarios/{name}`), `runtime_scenarios` with their names. A plain run has none of them.
- **`limit`**: the line used, as `{ "p99_ms", "error_rate" }`. The body takes a percentage (`error_pct: 1`); the record carries the fraction (`error_rate: 0.01`).
- **`points`**: every point run, sorted by `k`, each with `req_s`, `requests`, `goodput_req_s`, `error_rate`, `p50_ms`, `p95_ms`, `p99_ms`, `cause`, `over` (`{ "p99", "errors" }`: which of the two limits the point is over) and per-node `utilization`, `saturated`, `saturates_at_req_s`.
- **`first_to_saturate`**: the node saturated at the lowest `k`, with its analytic `saturates_at_req_s`. It is a report, not a cause: the knee can come from something else.
- **`dominant_cause`**: the most common failure cause at the first point over the limit.
- **`recovers_at`**: a `k` above the knee that passes again with margin (a retry storm or a breaker can do this).
- **`stopped_at`**: the `k` where the ladder was cut because a point was too large to run. With `holds_to` it means "held to the last point run", not "holds to the top".
- **`warnings`** and **`label`**, as in a run's summary. A sweep has no run log (`/log` is `409`).

From the CLI, `kaanal sim sweep` posts the sweep, waits for it and prints the finding:

```bash
kaanal sim sweep -p peak --p99 500ms --errors 1% --max-factor 8
```

`--p99` is a duration (`500ms`, `1s`; default 1s), `--errors` a percentage, `1%` or `1`, 0 to 100 (default 1%), `--max-factor` the top of the ladder (default 8); `--seed` and `--scenario` are as for `sim run`, and `-f FILE` takes a profile written out. A bad `--p99` or `--errors` is a usage error (exit 64) before anything is sent. The text is the verdict (`knee at 94 req/s (1.88x), the last point within the limit; the next point is over: p99 over 500ms`, or where it broke at the lowest point, or how far it held; a cut ladder says `held to N req/s, the last point run`, not `holds to`), the first node to saturate with its analytic figure, the dominant cause, the warnings (a cut ladder among them: `ladder stopped at 5.657x (283 req/s): run too large` when a point was too big to run, `ladder cut at ...` at the engine's rate cap) and the label, then one row per point: `load req/s`, `goodput`, `p99`, `errors` and `ok` (whether it passed the limit). `--no-wait` prints the id; `kaanal sim list` shows each record's `kind` (`run` or `sweep`); `kaanal sim show ID` prints a sweep record the same way (with its `seed`, `fingerprint` and `chaos in force` lines, what a re-run needs), and `--json` prints the record.

`kaanal sim report ID` prints the report of a finished sweep (Markdown; `--json` the data block alone, with `report_version`); `-o FILE` writes it instead and refuses to overwrite an existing file without `--force`. With `--force` the report is written to a new file that replaces the old one, so the file gets default permissions (your umask on Unix), not the old file's. `kaanal sim sweep ... --report FILE` writes the report when the sweep ends (an existing file is refused before the sweep starts, unless `--force`; `--report` with `--no-wait` is a usage error, exit 64). A run, a running or failed sweep, or an unknown id answers with the server's reason (`HTTP 409` or `404`), exit 1.

**The knee is only as true as the node figures.** The sweep finds where _the model_ breaks: if `proc`, slots, `cpu` or bandwidth are guesses, the knee is a guess too. Calibrate the slow nodes against a real measurement first, and read the knee as a relative figure (this design against that one), like every other result here.

## Sharing a sweep

A finished sweep has a **report**: one Markdown file that a person can read cold and an AI agent can parse. The server writes it, and the CLI and Studio only fetch it, so the bytes are the same everywhere. It holds, in order:

- **The label** first: _relative what-if, not a prediction_.
- **Finding:** the verdict (the knee, the last point within the limit, with what the next point is over; broken at the lowest point; or how far it held), the limit used, the first node to saturate with its analytic figure, the dominant cause, the **chaos in force** (`none`, `scenario NAME`, or `live chaos: active scenarios ...; N entries, digest X`), the **chaos digest** and the warnings.
- **Curve:** one row per point: `load req/s`, `goodput`, `p99 ms`, `errors` and `ok` (which limit a point is over).
- **Re-run:** the exact `kaanal sim sweep ...` command (profile, limits, `--max-factor`, `--seed`, `--scenario`), the seed, the design fingerprint and the Kaanal version. A profile written out (`-f FILE`) is embedded in the data block and the command reads it with `-f profile.yaml`. A sweep under live chaos adds a line saying the command alone cannot reproduce it (set the same chaos first: the count and digest let you check you did), and so does a sweep under a scenario defined at runtime (`--scenario=NAME` stays in the command, but no file holds that scenario, and after a restart the name is unknown); the chaos digest checks you set it up the same.
- **Data:** a fenced JSON block with the same data (`report_version`, `label`, `id`, `finding`, `points`, `recipe`; `recipe.chaos` is `none`, `scenario` or `live`, `recipe.live_chaos`, `recipe.chaos_digest` and `recipe.runtime_scenarios` the record's), for an agent or a script.

The report names only the nodes its finding mentions; it never carries the design file.

**The design fingerprint** is 12 hex digits of a SHA-256 over everything the build read: the config, each node's `spec:` file, the `endpoints:` file, `scenarios/*.yaml`, `locales/*.yaml` and the `data:` file or `data.yaml` (not the `site:` folder). Each file is parsed and written back with sorted keys before it is hashed, so comments, whitespace and key order do not change it. It covers the **design files only**, not the chaos a sweep ran under. That has its own **chaos digest**: 12 hex digits over the compiled chaos the sweep used, whatever it came from (a file, inline or runtime scenario, live entries), after the scenario choice and the stacking. **The same fingerprint, the same chaos digest and the same seed mean the same numbers; the fingerprint alone does not.** A different fingerprint or chaos digest means the two sweeps are not comparable. The chaos digest ignores the order live entries were set in and the config's node order; a different scenario stack order changes it only where it changes the effect (a higher scenario's failure comes first), so two stacks that compile to the same chaos, which give the same numbers, share it. What the fingerprint is **not**:

- Not a hash of the raw bytes: reformatting a file keeps it, while renaming a label changes it (a false "different" is harmless, a false "same" is not).
- Not the design on disk, and not every change that moves the numbers: it is computed when the system is built (each Apply), so it names the design last applied. A saved but unapplied edit does not change it, nor does live chaos, nor a parameter edit that went straight into a live run without Apply ([control-api.md](control-api.md)).
- Not a second reading: every file is hashed from the text the build read (a spec from the document it parsed), so a file saved while the build runs cannot be counted in a newer form than the one built.
- Not the Kaanal version, which the recipe names on its own line.

`kaanal sim show ID` prints the fingerprint and the seed of a sweep.

From the CLI:

```bash
kaanal sim report s2                 # the Markdown on stdout
kaanal sim report s2 --json          # the data block alone
kaanal sim report s2 -o sweep.md     # write a file; --force to overwrite one
kaanal sim sweep -p peak --report sweep.md   # sweep, then write the report
```

Over HTTP, `GET /__kaanal/sim/runs/{id}/report` answers `text/markdown`, and `?format=json` the data alone. It is `404` for an unknown id or one pushed out of the last 8 records, and `409` while the sweep runs, for a failed sweep and for a plain run. See [control-api.md](control-api.md).

```bash
curl -s localhost:8080/__kaanal/sim/runs/s2/report -H "Authorization: Bearer $TOKEN"
```

**In Studio**, the Results tab has a group **Find where it breaks**: a p99 limit (ms, default 1000) and an error limit (%, default 1), and **Start**. While it runs it says "point N of up to M" and the points arrive on a chart (load factor and p99 on log axes, the limit as a dashed line, the knee as a labelled line, a point over a limit drawn as a square, an error-rate strip under it, and a points table). When it ends it shows the finding, the first node to saturate, the dominant cause and the warnings, and two buttons: **Copy as Markdown** and **Save as a file** (`kaanal-sweep-<id>.md`). Studio fetches the report once, when the sweep ends, and keeps that text, so it is still there after the server has dropped the record. On Windows the clipboard can turn the line ends into CRLF; the text is otherwise the CLI's.

- Studio sweeps the **applied** design with its first `traffic:` profile. While the draft has unapplied edits it says "Sweeps the applied design; Apply to include your edits."
- A sweep made before the design was applied again says "Made on an earlier design", and one made before a server restart says so too: start a new sweep to see the design in force.
- In Live mode the group is there but Start is off: a sweep steps the load, which a live system cannot. Switch to Batch.
- A sweep and a run share the one slot: while a run is in progress Start is off, and a slot taken elsewhere says the simulator is busy.

## Sizing a design: what would hold this load?

A sweep says where the design breaks. A **sizing plan** answers the next question: "what do I change so it holds 3x this traffic, and what does that cost?" It is a **relative what-if, not a prediction**: the same cost model as every run, so it ranks changes and shows their order of size; it does not promise a number, and every price is an estimate. Kaanal never applies a plan. It writes a **proposed draft**, a text you read and apply yourself.

```bash
kaanal sim plan -p peak --target 3x --report plan.md -o plan.yaml
```

- **The target** is a multiple of the profile's rates (`3x`, 0.1 to 64) or a total rate (`500`, req/s, shaped like the profile). The limits are the sweep's: `--p99` (default 1 s) and `--errors` (default 1%). A design **holds** the target only with a margin: p99 at most 90% of the p99 limit and errors at most half of the error limit, so a plan does not sit on the edge.
- **The check** is one run at the target load (not a sweep). It gives pass or fail, the p99 and error rate, and a **capacity estimate**: the lowest closed-form saturation rate of any node, as in the sweep. The estimate is not a measurement.
- **The walk** repeats the check and moves the first node to saturate one size step, until the design holds, nothing sizable is left, the cause is one size cannot fix (see _Cannot hold it_), the next step would add to the price and break the budget (a step that adds nothing never does), two steps in a row do not help, or `--max-steps` (default 30, 1 to 100) is reached. Each step shows the node, the change, the price, the capacity estimate and the p99 and error rate at the target, before and after.
- **What it may change** (size knobs only): the instance class of a node up the same series in the region's price file (`m5.large` to `m5.xlarge`), cores only on a node with no instance, memory only when the failures at the target are out-of-memory (`oom`: the next class up on a node with an instance, else its written memory limit doubled), and copies (`replicas`). A cache gets copies only. It never changes a node's kind, the region, the wires, a rate limit, a timeout, a breaker or a retry. A node you do not want touched is kept with `--keep NODE` (repeatable).
- **The menu** lists, at each step, the other moves the walk tried for that node (a larger class, one more copy) with their price and effect. No extra runs.
- **Cannot hold it** is a result, with the reason. When the errors at the target are over the limit and most failures come from a rate limit you set (the node is named when one node has one), from the chaos in force, or from a deadline you set while no node saturates, the walk sizes nothing and says sizing cannot fix it. When the bottleneck has no size knob (an external API, an object store, a node you kept), the verdict names it the same way. The node can be absent in the data (`null`) when the cause is not one node's. The plan stops rather than invent a fix.
- **The budget** (`--budget 2.00`) is a ceiling in dollars an hour. It counts the **priced nodes only**: a node with no `instance:` has no price and is listed as not priced. It includes per-use prices at the visits of the target run: the walk prices without them, so the verdict is judged again from the re-check's cost, and a **budget check** line says what the plan shown costs against the budget (and says so when those prices turned a holding plan into an over-budget one). A plan that holds but costs more is not hidden: the verdict says it holds and is over budget by how much. When no holding design was found, the verdict says so, what the next step needed would cost and by how much that passes the budget; the budget check says whether the plan shown is itself within the budget (it is not when the design in force already costs more).
- **Dollars are dollars an hour.** In the data, fields named `micro_usd_hr` are millionths of a dollar an hour (1,000,000 = one dollar), integers.

**If you changed the database family** is a separate section, never part of the path. For a database whose `ops:` you did not write, it runs the same walk as if the flavor were the other family (relational against `mongodb`, `dynamodb`, `cassandra`, and the reverse) on the flavor's preset read, write and scan times. It **assumes your application can use that family** (no joins, transactions or consistency are modelled), the presets are estimates, and a price appears only where a price table exists. It is a what-if, not a recommendation to migrate. A database with `ops:` of your own, or one you kept, is skipped with the reason.

**Cheapest, and the provider what-if.** The report of a cheapest plan (`cheapest: true` over HTTP) reads differently from a walk's, on purpose. Its label is _cheapest found within the limit, not proven cheapest_: the search tries smaller sizes of each priced node (one with no `instance:` has no price and is never shrunk) and keeps one only while the design still holds, and it assumes a bigger size never holds less, so a cheaper design can exist that it did not reach. **Search** says why it stopped (it ran out of candidates, reached the run cap, or its time limit passed), how many runs it used of the cap, and the time it used of its share of the limit. When the design in force already holds and the plan ends cheaper, it says the design "could cost less" with both prices. **Plan** lists the changes, each with its own price effect and no running price: the search merges the moves on one knob of one node, so the design between two lines is not one it checked. A run cap or time limit that stopped the search before any holding design was found says none was found _within the limit_, which does not show that none exists. **Comparison** (when provider sections ran) puts the design in force and each provider's translated design in one table, the ones that hold ranked cheapest first with the winner named; if none holds at any size tried there is no winner, and a skipped provider is listed with its reason. Each provider section says the translation assumes the wires, egress, data location and application are unchanged and that prices are on-demand estimates; it is a separate what-if with its own search, never a mixed design, and it has no draft. A node you keep with `--keep` that has a provider's class would stay on that provider while the rest moved, so every section is skipped with the node named ("db: kept on aws, so a section on gcp would mix providers"); a cache on a class has no translation and is treated the same way ("c: a cache on aws has no translation, so a section on gcp would mix providers"); a kept node with no class or with your own instance has no provider and stays as written (the section lists it). With no provider section there is no comparison and no winner. The record carries the comparison (`comparison`) so the Markdown report and `kaanal sim plan` print the same rows and the same closing sentence. A saving under a cent is printed as it is (`$0.0035/h`), not as `$0.00`.

**Which databases move to which provider.** A node with a compute class (a service, a VM, a self-hosted database) moves to any provider. A **managed** database moves only where the price table has a managed class for its flavor, which today is:

| flavor                                                       | to AWS (RDS)                                              | to Azure                        | to GCP |
| ------------------------------------------------------------ | --------------------------------------------------------- | ------------------------------- | ------ |
| `sqlserver`                                                  | yes (RDS SQL Server, the engine you wrote, else Standard) | yes (Azure SQL Database, vCore) | no     |
| `postgres`, `mysql`, `mariadb`, `oracle`, `mongodb`, `neo4j` | yes (already AWS classes)                                 | no                              | no     |
| `cassandra`, `dynamodb`                                      | no managed class                                          | no                              | no     |

GCP Cloud SQL and Azure Database for PostgreSQL and MySQL are not in the price table yet, so a design with a managed PostgreSQL or MySQL gets no GCP or Azure section: it is skipped with what is missing ("db: no managed postgres class for gcp in the price table"), listed first under **Comparison**. A managed Azure SQL database with no `flavor:` is not moved to AWS (its engine there is unknown; write `flavor:`).

**The time limit and the flags.** `kaanal sim plan --cheapest` takes `--time` (default 150s, 1s to 600s), `--run-cap` (default 200 runs per search, 1 to 2000) and `--providers gcp,azure` or `--providers none` (default: every other provider when the design has a provider class; `none` leaves out the provider sections). The limit bounds the **searches** only: with provider sections, half of it goes to the main search and the rest is shared equally between the sections, and it is checked between runs, so a search ends within its share plus the one run in progress. The re-check, the pricing runs of the sections and the sweep of the proposed draft run **after** the search and are **not counted**, so the whole job can take longer than `--time`. In cheapest mode there is **no database-family section** (`families` is empty); the provider what-if takes its place, and the family what-if is the walk's, without `--cheapest`. A node kept with `--keep` is not changed by the search.

**Checked.** The proposed draft is built again from its own text and run at the target, so what is reported holding is what the file says. A load sweep of that design then finds its knee, which should lie above the target. The checked design's cost counts per-use prices at that run's visits, so it can differ a little from the plan's price.

The report (`kaanal sim report ID`, or `--report`) has the label first, then the target, now, the steps, the menu, the family section, the verdict, the check, a re-run command and a JSON `## Data` block. It carries the design fingerprint, the chaos digest and the seed: the same three and the same target give the same plan, byte for byte apart from the id and, for a cheapest search, the time it reports as used, which is a wall-clock measure (comments in the file do not change the fingerprint). A plan is of the **applied** design, as a sweep is; Apply a draft first. It shares the run slot with runs and sweeps (there is no Studio view of it yet). Over HTTP: `POST /__kaanal/sim/plan`, then `GET /__kaanal/sim/runs/{id}/draft` for the proposed draft; see [control-api.md](control-api.md).

**In Studio**, the **Plan** button in the top bar (beside the mode switch) opens a wide window, in every mode. A plan is of the applied design whatever the live stream is doing, and the window says so: a live edit that is not applied yet is not in it, and if a live edit has changed the design in force the server refuses with its own reason (the design in force differs from the applied text). Type a target (times the profile's rates, or req/s), the p99 and error limits, an optional budget and the nodes to keep as they are; tick **Also look for the cheapest design** to set a time limit, a run cap and the providers to compare. **Start** plans the applied design, not the draft; while it runs the steps appear as the walk takes them. The result is the report's own: the verdict sentence, the changes with each step's price effect and, under each, the other moves tried; the comparison table when provider sections ran; the database-family what-ifs (folded); and what the proposed draft was checked at, with the sweep's knee. The sentences are the server's (the record carries them as `words` and `comparison`), so the window, the Markdown report and `kaanal sim plan` never word a verdict differently. **Copy as Markdown** and **Save as a file** give the report. Closing the window keeps the plan and a running search goes on; the **Plan** button says "running", "done", "earlier design" (the design was applied again or the server restarted since) or "failed". **Load proposed draft** puts the proposed design in the Studio draft, and you press Apply yourself. If the draft has unsaved edits it asks first and cannot bring them back with Undo; **Copy my draft** saves them to the clipboard first. A plan that could not hold the target has no proposed draft, so no Load.

## Limits

- **One run at a time.** A second `POST` while one runs is `409` with the running id. A sweep or a plan counts as a run.
- **Server caps.** A run is limited to about 2 million recorded hops and 5 million simulated hops (a couple of seconds of CPU); a bigger request is `400` and says which limit it hit. Lower the rate or the duration.
- **A run keeps the system it began on.** If you Apply a new draft while it runs, the report still describes the old one; its `generation` says which.
- The report is kept, the per-request records are not.

## Reading the report

The record also has **`cost`** (RULES 126): what the system costs an hour, estimated with the same sums as the cost card in Studio: `total_micro_usd_hr` (micro-USD, 1,000,000 = 1 dollar), `priced` and `unpriced` node counts, each node's `unit_micro_usd_hr`, `total_micro_usd_hr`, `copies`, `instance` and a `why` when it has no price (serverless, external, no instance, no price in the region), and the `regions` each cloud was priced in (the file's `region:`, else the default). An object store's requests are priced at this run's visits. `kaanal sim show` prints it as one line. It is an estimate for a what-if, never a quote; the runs list leaves it out like the summary.

`summary` holds:

- **`latency`**: `p50_ms`, `p95_ms`, `p99_ms`, `max_ms` and log2 buckets. Percentiles are exact over the run's requests.
- **`requests`, `failed`, `errors`**: reported requests, how many failed, and failures by kind (`status`, `reset`, `malformed`, `timeout`).
- **`nodes`**: per node, with a row per replica: `visits_per_s` (measured) beside `expected_visits_per_s` (what the chains predict), `mean_wait_ms`, `offered_load`, `utilization`, `saturated`, `max_queue`, and refusals (`limited`, `overflowed`, `timed_out`) and `short_circuited` (calls a caller's open circuit breaker refused without sending, a 503 with the cause `breaker`); `capacity_per_s` (what the node can take in **visits** per second, not requests per second: slots × replicas ÷ mean service time per visit, absent without a limit or visits), `saturates_at_req_s` (the same figure in **requests** per second for the whole traffic mix: `capacity_per_s` divided by the visits one request makes to the node; absent when there is no capacity or no request reaches the node); `headroom` (`1 - offered_load`, at least 0); and `series`: 60 windows of `busy` (fraction of slots busy), `queue` (the longest queue seen when a request joined, taken per replica, so it misses requests cancelled while queued and can read low under deadlines) and `latency` (counts of the node's own hop durations in the summary's buckets), `window_s` wide each. A run with a warmup under-counts the earliest windows. In the run log the `queued` event fires only when the wait is longer than the service time.
- **`pipes`**: per node `bandwidth` side (`send` and `receive`, only the sides with a rate): `node`, `direction`, `transfers` (bodies for which this pipe was the narrowest), `mean_wait_ms`, `offered_load` (transfer time over the window and the node's copies), `utilization`, `saturated` and `saturates_at_req_s`, as for a wire. No per-window series.
- **`links`**: per edge with a `bandwidth`: `transfers`, `series` (60 windows of `latency`: counts of the time a body spent on the wire, the transfer plus the time lost while other bodies shared the wire, in the same buckets as `latency.buckets`, by the window it joined the wire in; no busy share, because the log's records leave out the requests a full wire spends the run serving), `mean_wait_ms`, `offered_load`, `utilization`, `saturated` (offered load 1 or more, or utilization 0.95 or more) and `saturates_at_req_s` (requests per second for the whole traffic mix before this wire is full: the mix rate divided by `offered_load`; absent when nothing crossed the wire). A full wire slows every body on it, so the nodes behind it can sit idle while its transfers share it.
- **`bottlenecks`**: node ids, most loaded first (nodes with a `concurrency` or `pool` limit first, since a node with no limit cannot queue).
- **`streams`**: percentiles per endpoint.
- **`warnings`**: read these first.

**Offered load and utilization.** `offered_load` is hop time over capacity (for a node with `hold: call`, the time its slots were held, which includes the wait on what it called) and can pass 1; `utilization` is the share of slot time actually busy and tops out at 1. A node is `saturated` when its offered load is 1 or more, its utilization is 0.95 or more, or it already refuses work (a full queue or a fired `timeout:`). Its queue then grows for as long as the run lasts (with a queue limit it fills and the node turns requests away instead), so its latency is not a steady-state figure.

**CPU utilization.** For a node with `cpu` the summary also has `cpu_utilization`: the CPU the work really used in the window over cores x copies x window, at most 1 (the engine's busy area; measured, where `cpu_load` below is offered). **CPU load.** For a node with `cpu` the summary also has `cpu_load` (its own work, at the drawn times, over cores x copies x window; a callee wait inside a `hold: call` slot is not CPU), `cpu_capacity_per_s` (cores x copies over the mean own work per visit) and `saturates_by`. `saturates_at_req_s` is then the lower of the slot and CPU capacities and `saturates_by` says which (`slots` or `cpu`). `cpu_load` of 1 or more marks the node saturated even without slots, and the warning adds `CPU load`. These are offered figures: there is no measured CPU utilisation yet. See [CPUs](nodes.md#cpus-cpu).

**Warnings about noise.** With fewer than 1000 reported requests the p99 is noise; with fewer than 100, so is p95. One run is one sample of a random process and has no confidence interval: run longer, or run twice with different seeds and compare.

## The run log

`GET /__kaanal/sim/runs/{id}/log` pages a **sample** of the run's requests, one line per hop, in virtual time (`t_ms` is the arrival plus the hop's start). It is not every request: it keeps every failed request (up to 1000), the 500 slowest, and an even spread of the rest, 4000 requests at most and 20 000 lines in all (a run that reaches the cap says so in `note`). Only requests that arrived after the warmup are logged.

Query: `node`, `from` with `to` (one wire: only the hops on the wire from one node to another, so only a wire with a `bandwidth`; one of the two alone is 400), `replica`, `endpoint` (`GET /users/{id}`, URL-encoded), `level` (`info`, `warn`, `error`; anything else is 400), `limit` (default 200, at most 1000), `after` (the `next` of the previous page). Filters combine with AND; an unknown `node` answers 200 with no lines. The answer is `{ lines, next, total, sampled: true, note }`: pass `next` as `after` to continue, `null` means the end; `total` counts the stored lines before filtering.

A line has `t_ms`, `level`, `request` (the endpoint's request counter), `stream` (the endpoint), `node`, `from` (only on a hop on a wire, `link request` or `link response`: the node that called over it, so the wire is `from` to `node`), `replica`, `event`, `wait_ms`, `service_ms`, `status`, `note` and, on a hop that ran in a retry, `attempt` (2 or more; absent in a first attempt, so it is also absent from every run without retries). `event` is `served`, `queued` (the wait was longer than the service: level `warn`), or the failure of a request, on the failing node at the instant it failed (level `error`): `failed` (chaos), `limited` (a rate limit), `overflowed` (a full queue), `timeout` (a deadline). The event comes from the simulator's own record of the cause, so a chaos 503 on a node whose full queue also answers 503 is still `failed`. The simulator does not record which replica a failure happened on, so a failure line has `replica`, `wait_ms` and `service_ms` set to `null`, and `?replica=N` skips failure lines. The log never holds half a request: when the 20 000-line cap is reached, the even spread of requests gets thinner (still across the whole run) and `note` says so.

## Sample a body

`POST /__kaanal/sample` with `{ "endpoint": "GET /users/{id}", "seed": 7 }` returns the body the mock would generate, its size in bytes and the success status, so you can see what a route carries. It is the same generator and seeds the mock uses. A route without a response schema returns `body: null`; a paged list route returns the whole schema body, not one page; a sample over 256 KiB is refused. The size is what a journey's `resp_bytes` defaults to.

## A spec for a system that has none

An `endpoints:` entry cannot create a route by itself: routes belong to a node through its OpenAPI `spec:`. If you have designed a system and its endpoints but have no spec, `GET /__kaanal/openapi/skeleton` (or `kaanal skeleton`) writes one: an OpenAPI 3.0 file with an operation for every key under `endpoints:` in the **draft**, its path parameters (`in: path`, `required: true`), a success response (the entry's `override` status, else 200) with an empty object schema, and `x-kaanal-skeleton: true`. It reads the draft, so it works before the first Apply.

```bash
kaanal skeleton -o specs/app.yaml        # refuses to overwrite without --force
# then set  nodes.app.spec: specs/app.yaml  in the draft and apply it
```

Paths are written as they are (a catch-all `{*rest}` stays one, since the router reads spec paths unchanged), repeated parameters are declared once, and a skeleton covers at most 10000 endpoints. The skeleton lists every key in `endpoints:`, because an entry does not say which node owns it; use it on a node that has no spec of its own. Payload sizes still come from the traffic profile.

## What the figures leave out

Two real limits are off unless you turn them on, so a healthy-looking run can be optimistic. With `hold: own` (the default) a request frees its slot while it waits on what it calls; real servers that block on a call (`hold: call`, Studio's Server style) keep it. And without a `bandwidth` on a node or a wire, bodies cross the network for free. When a run has components that call each other and neither is set, Results says so in a quiet grey note; set the one that fits your system and run again. A real system may saturate earlier than the run shows.

## Known limits

- **A journey is a stream of independent requests**, not a user session: no think time, no state, and Poisson arrivals are smoother than real traffic. Real tails will be worse.
- **One run is one sample.** There is no confidence interval; the report warns when the tail rests on few requests. Run longer or compare seeds.
- **No cancel.** A run is bounded to a couple of seconds of CPU by the server caps, so it ends on its own. A sweep is a run per point (11 ladder points at the default `max_factor`, plus at most 6 to refine), each under the same caps. Runs are not kept across a restart (the last eight are kept while the server runs).
- **Request bodies are not sized from the spec.** `req_bytes` is 0 unless the profile sets it. A paged list route's default `resp_bytes` is the whole schema body, not one page.
- **Samples** use the mock's generator and seeds, but a sample's data can differ from what the mock returns for the same path.
- **Skeleton specs** have empty object schemas and cover every key of `endpoints:`.
- **Not built yet:** profiles in `traffic/NAME.yaml` files (use the `traffic:` section), live load against the running mocks, and adding the generated skeleton to the draft in one step (save it with `kaanal skeleton -o` and set `spec:`).
- **Seeds.** A run uses the request's `seed`, else the config's `seed:`. Under `determinism: realistic` the live mocks use a seed drawn at startup, so a run is repeatable but its draws do not match what the mocks did.
- **Waits under timeouts are biased low.** A request that a `timeout:` cancels while it still queues never starts its hop, so it is absent from `mean_wait_ms` and `visits_per_s` on a node that sheds load. Utilization and the saturation flag do not have this bias.
- **Pins are not part of a run.** A pinned UI state (`kaanal cycle`) changes what the mocks answer, not what the simulator runs; only the chaos does.
- **Payload sizes follow the mocks, with one difference:** a `HEAD` counts as 0 bytes, as live mode charges it. A `collection:` route and a built-in responder without an `override` also count as 0 here, because a run has no stored items and has no credentials to size a login with; the live mocks size a collection route from what it holds right now, the JWKS route from its key set, and a login from the account's token (measured, not signed).
