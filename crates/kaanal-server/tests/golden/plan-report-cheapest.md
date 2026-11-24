# Sizing plan s4

*relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro\_usd are millionths of a dollar), of the priced nodes only*

This plan is not a prediction. It is what the model says about this design at the load below, and every price is an estimate. Kaanal does not apply it: the proposed draft is a text for you to read and apply yourself (`GET /__kaanal/sim/runs/s4/draft`; it is not in this report).

*cheapest found within the limit, not proven cheapest*

## Target

- Offered load: 250 req/s, 2.50x the rates of profile peak.
- Limit: p99 1s, errors 1%. A design holds the target only with a margin: p99 at most 90% of the p99 limit and errors at most half of the error limit.
- Budget: $2.00/h, counted over the priced nodes only.
- Kept as they are: cache.
- Most steps: 30.

## Now

The design in force, at that load: p99 300 ms, errors 0%, capacity est. 251 req/s. It costs $1.62/h (priced nodes only).

## Search

- Stopped because it ran out of candidates to try.
- Runs: 64 of 200 allowed.
- Time: 31s 800ms used of 1m 15s, this search's share of the 150 s time limit, which is split between it and each provider section.

The design in force already holds the target and could cost less: $1.62/h now, $0.9265/h with the changes below, $0.6935/h less.

The search tries smaller sizes of each priced node and keeps one only if the design still holds the target with the margin. It assumes a bigger size never holds less, which is not always so, so a cheaper design can exist that it does not reach.

## Comparison

Not compared:

- Azure: db: no managed postgres class for azure in the price table

The designs that hold the target come first, cheapest first; the ones that do not come after them with how they ended. The price is the design's cost an hour with per-use prices counted at one run's visits at the target, the figure its own section states: an on-demand estimate that leaves out the nodes with no price (the column counts them). A design that does not hold runs saturated, so its per-use prices are understated. A provider row is the design translated to that provider's classes, searched on its own.

| rank | design | price an hour | nodes not priced | holds the target | how it ended |
| ---: | --- | --- | --- | --- | --- |
| 1 | the design translated to GCP | $0.892/h | 1 | yes | holds |
| 2 | the design in force, with the changes above | $0.94/h | 1 | yes | holds |

Winner: the design translated to GCP, $0.892/h. It is the cheapest design that holds the target found within the limit, not proven cheapest.

## Plan

These are the changes the search found, in the order it found them: first what the walk needed to make the design hold, then what the search took off. Each change is one size knob of one node. The price effect is what that change adds or takes off, an estimate; the moves on one knob of one node are merged into one line, so the figures of the design between two lines are not shown. The figures are what the model gives at the target load.

| # | node | change | price effect | note |
| ---: | --- | --- | --- | --- |
| 1 | app | copies 5 to 3 | -$0.40/h | cheaper by $0.40 an hour and still holds |
| 2 | db | instance class db.m5.2xlarge to db.m5.xlarge | -$0.29/h | cheaper by $0.29 an hour and still holds |
| 3 | worker | instance class c5.xlarge to c5.large | -$0.0035/h | cheaper by $0.0035 an hour and still holds |

The design in force before any change: p99 300 ms, errors 0%, capacity est. 251 req/s; $1.62/h. After the changes: p99 520 ms, errors 0.1%, capacity est. 255 req/s; $0.9265/h.

## Verdict

The plan holds the target.

Budget check, with per-use prices counted at the target run's visits: the plan shown costs $0.94/h, within the budget of $2.00/h.

Not priced: cdn. They are left out of every price and of the budget.

## Checked

The proposed draft was built from its text and checked again at the target: p99 520 ms, errors 0.1%, capacity est. 255 req/s. It holds the target with the margin.

Its cost, with per-use prices counted at that run's visits: $0.94/h (3 nodes priced, 1 not). It can differ from the plan's price above, which has no per-use part.

A load sweep of the proposed design finds its knee near 262 req/s (2.62x the profile's rates), to the sweep's search resolution: the last point within the plain limit (no margin) before the first one over it.

## If it ran on GCP

*cheapest found within the limit, not proven cheapest*

This what-if assumes wires, egress, data location and application are unchanged; on-demand price estimates.

The plan holds the target.

Budget check, with per-use prices counted at the target run's visits: the plan shown costs $0.892/h, within the budget of $2.00/h.

- Stopped because it reached the run cap, so cheaper designs may be left untried.
- Runs: 200 of 200 allowed.
- Time: 22s 400ms used of 37s 500ms, this section's share of the time limit.

The translation:

- app: aws/m5.large to gcp/n2-standard-2
- nginx: kept as written, not compared (no class)

| # | node | change | price effect | note |
| ---: | --- | --- | --- | --- |
| 1 | app | copies 5 to 3 | -$0.60/h | cheaper by $0.60 an hour and still holds |

The translated design before any change: p99 300 ms, errors 0%, capacity est. 251 req/s; $1.48/h. After the changes: p99 540 ms, errors 0.1%, capacity est. 250 req/s; $0.88/h.

## Re-run

```bash
kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --cheapest --time=150s --run-cap=200 --providers=gcp,azure --seed 7
# seed 7: every candidate draws the same random numbers, so the same seed on the same design repeats the plan
# fingerprint 0123456789ab: SHA-256 of the design files the build read, comments and whitespace aside; it does not cover the chaos in force (here: none; chaos digest 9f86d081884c): the same fingerprint, the same chaos digest and the same seed mean the same numbers, the fingerprint alone does not
# kaanal 9.9.9
```

## Data

```json
{
  "id": "s4",
  "label": "relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro_usd are millionths of a dollar), of the priced nodes only",
  "plan": {
    "budget_check": {
      "budget_micro_usd_hr": 2000000,
      "cost_micro_usd_hr": 940000,
      "rejudged": false
    },
    "cheapest_note": "cheapest found within the limit, not proven cheapest",
    "end": {
      "capacity_req_s": 255.0,
      "error_rate": 0.001,
      "p99_ms": 520.0
    },
    "families": [],
    "label": "relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro_usd are millionths of a dollar), of the priced nodes only",
    "measured_knee": {
      "k": 2.62,
      "kind": "knee",
      "req_s": 262.0
    },
    "price_after": 926500,
    "price_before": 1620000,
    "providers": [
      {
        "budget_check": {
          "budget_micro_usd_hr": 2000000,
          "cost_micro_usd_hr": 892000,
          "rejudged": false
        },
        "cost": {
          "egress": [],
          "nodes": [],
          "priced": 3,
          "regions": {},
          "total_micro_usd_hr": 892000,
          "unpriced": 1
        },
        "info": {
          "design_in_force_holds": true,
          "elapsed_ms": 22400,
          "limit_ms": 37500,
          "notes": [],
          "run_cap": 200,
          "runs": 200,
          "stop": "run_cap"
        },
        "kind": "answer",
        "notes": [
          "app: aws/m5.large to gcp/n2-standard-2",
          "nginx: kept as written, not compared (no class)",
          "assumes wires, egress, data location and application are unchanged; on-demand price estimates",
          "cheapest found within the limit, not proven cheapest"
        ],
        "plan": {
          "end": {
            "capacity_req_s": 250.0,
            "error_rate": 0.001,
            "p99_ms": 540.0
          },
          "families": [],
          "price_after": 880000,
          "price_before": 1480000,
          "start": {
            "capacity_req_s": 251.0,
            "error_rate": 0.0,
            "p99_ms": 300.0
          },
          "steps": [
            {
              "after": {
                "capacity_req_s": 250.0,
                "error_rate": 0.001,
                "p99_ms": 540.0
              },
              "before": {
                "capacity_req_s": 251.0,
                "error_rate": 0.0,
                "p99_ms": 300.0
              },
              "menu": [],
              "mv": {
                "kind": {
                  "from": 5,
                  "knob": "copies",
                  "to": 3
                },
                "node": "app"
              },
              "note": "cheaper by $0.60 an hour and still holds",
              "price_delta_micro_usd_hr": -600000,
              "price_micro_usd_hr": null
            }
          ],
          "unpriced_nodes": [],
          "verdict": {
            "kind": "holds"
          }
        },
        "provider": "gcp"
      },
      {
        "kind": "skipped",
        "provider": "azure",
        "reason": "db: no managed postgres class for azure in the price table"
      }
    ],
    "request": {
      "budget_micro_usd_hr": 2000000,
      "cheapest": {
        "providers": [
          "gcp",
          "azure"
        ],
        "run_cap": 200,
        "time_limit_s": 150
      },
      "factor": 2.5,
      "keep": [
        "cache"
      ],
      "limit": {
        "error_rate": 0.01,
        "p99_ms": 1000.0
      },
      "max_steps": 30,
      "req_s": 250.0,
      "target": {
        "multiple": 2.5
      }
    },
    "search": {
      "design_in_force_holds": true,
      "elapsed_ms": 31800,
      "limit_ms": 75000,
      "notes": [],
      "run_cap": 200,
      "runs": 64,
      "stop": "exhausted"
    },
    "start": {
      "capacity_req_s": 251.0,
      "error_rate": 0.0,
      "p99_ms": 300.0
    },
    "steps": [
      {
        "after": {
          "capacity_req_s": 255.0,
          "error_rate": 0.001,
          "p99_ms": 520.0
        },
        "before": {
          "capacity_req_s": 251.0,
          "error_rate": 0.0,
          "p99_ms": 300.0
        },
        "menu": [],
        "mv": {
          "kind": {
            "from": 5,
            "knob": "copies",
            "to": 3
          },
          "node": "app"
        },
        "note": "cheaper by $0.40 an hour and still holds",
        "price_delta_micro_usd_hr": -400000,
        "price_micro_usd_hr": null
      },
      {
        "after": {
          "capacity_req_s": 255.0,
          "error_rate": 0.001,
          "p99_ms": 520.0
        },
        "before": {
          "capacity_req_s": 251.0,
          "error_rate": 0.0,
          "p99_ms": 300.0
        },
        "menu": [],
        "mv": {
          "kind": {
            "from": "db.m5.2xlarge",
            "knob": "class",
            "to": "db.m5.xlarge"
          },
          "node": "db"
        },
        "note": "cheaper by $0.29 an hour and still holds",
        "price_delta_micro_usd_hr": -290000,
        "price_micro_usd_hr": null
      },
      {
        "after": {
          "capacity_req_s": 255.0,
          "error_rate": 0.001,
          "p99_ms": 520.0
        },
        "before": {
          "capacity_req_s": 251.0,
          "error_rate": 0.0,
          "p99_ms": 300.0
        },
        "menu": [],
        "mv": {
          "kind": {
            "from": "c5.xlarge",
            "knob": "class",
            "to": "c5.large"
          },
          "node": "worker"
        },
        "note": "cheaper by $0.0035 an hour and still holds",
        "price_delta_micro_usd_hr": -3500,
        "price_micro_usd_hr": null
      }
    ],
    "unpriced_nodes": [
      "cdn"
    ],
    "verdict": {
      "kind": "holds"
    },
    "verify": {
      "capacity_req_s": 255.0,
      "cost": {
        "egress": [],
        "nodes": [],
        "priced": 3,
        "regions": {},
        "total_micro_usd_hr": 940000,
        "unpriced": 1
      },
      "error_rate": 0.001,
      "holds": true,
      "p99_ms": 520.0
    }
  },
  "recipe": {
    "chaos": "none",
    "chaos_digest": "9f86d081884c",
    "command": "kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --cheapest --time=150s --run-cap=200 --providers=gcp,azure --seed 7",
    "fingerprint": "0123456789ab",
    "generation": 2,
    "inline_profile": null,
    "kaanal_version": "9.9.9",
    "live_chaos": {
      "digest": "e3b0c44298fc",
      "entries": 0,
      "scenarios": []
    },
    "profile": "peak",
    "runtime_scenarios": [],
    "scenario": null,
    "seed": 7
  },
  "report_version": 1
}
```
