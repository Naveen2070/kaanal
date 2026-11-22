# Sizing plan s4

*relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro\_usd are millionths of a dollar), of the priced nodes only*

This plan is not a prediction. It is what the model says about this design at the load below, and every price is an estimate. Kaanal does not apply it: the proposed draft is a text for you to read and apply yourself (`GET /__kaanal/sim/runs/s4/draft`; it is not in this report).

## Target

- Offered load: 250 req/s, 2.50x the rates of profile peak.
- Limit: p99 1s, errors 1%. A design holds the target only with a margin: p99 at most 90% of the p99 limit and errors at most half of the error limit.
- Budget: $2.00/h, counted over the priced nodes only.
- Kept as they are: cache.
- Most steps: 30.

## Now

The design in force, at that load: p99 4100 ms, errors 5%, capacity est. 49 req/s. It costs $0.84/h (priced nodes only).

## Plan

Each step moves one size knob of one node (instance class, cores, memory limit or copies). The figures are what the model gives at the target load before and after the step. Capacity est. is the lowest closed-form saturation rate of any node: an estimate, not a measurement.

| step | node | change | price | capacity est. req/s | p99 at target ms | errors at target |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | db | instance class db.m5.large to db.m5.2xlarge | +$0.58/h; $1.42/h in all | 49 to 118 | 4100 to 1200 | 5% to 0.2% |
| 2 | app | copies 2 to 3 | +$0.20/h; $1.62/h in all | 118 to 251 | 1200 to 300 | 0.2% to 0% |

Note on step 2: a second copy of app halves its queue

The design the plan ends in: p99 300 ms, errors 0%, capacity est. 251 req/s; $1.62/h (the design in force costs $0.84/h).

## Menu

At each step the walk tried every move the bottleneck node offers and took the one marked taken. The price is the whole design's with that move; the figures are at the target load.

Step 1, node db:

- instance class db.m5.large to db.m5.xlarge: $1.13/h; p99 2900 ms, errors 1.5%, capacity est. 83 req/s; does not hold the target.
- instance class db.m5.large to db.m5.2xlarge (taken): $1.42/h; p99 1200 ms, errors 0.2%, capacity est. 118 req/s; does not hold the target.
- copies 1 to 2: $1.09/h; p99 3300 ms, errors 2%, capacity est. 77 req/s; does not hold the target.

Step 2, node app:

- copies 2 to 3 (taken): $1.62/h; p99 300 ms, errors 0%, capacity est. 251 req/s; holds the target.
- instance class c5.large to c5.xlarge: $1.70/h; p99 800 ms, errors 0.1%, capacity est. 160 req/s; holds the target.

## If you changed the database family

These are what-ifs, not a recommendation to migrate. Every answer assumes your application can use this family; preset estimates. The plan above does not depend on them.

- db as dynamodb:
  - The plan holds the target.
  - 1 step taken, ending at: p99 280 ms, errors 0%, capacity est. 300 req/s; $1.20/h (the changed design costs $0.70/h before any step).
  - the table is priced as provisioned throughput
  - assumes your application can use this family; preset estimates
- db as mongodb: not walked: no preset instance table for this flavor

## Verdict

The plan holds the target.

Budget check, with per-use prices counted at the target run's visits: the plan shown costs $1.6345/h, within the budget of $2.00/h.

Not priced: cdn. They are left out of every price and of the budget.

## Checked

The proposed draft was built from its text and checked again at the target: p99 300 ms, errors 0%, capacity est. 251 req/s. It holds the target with the margin.

Its cost, with per-use prices counted at that run's visits: $1.6345/h (3 nodes priced, 1 not). It can differ from the plan's price above, which has no per-use part.

A load sweep of the proposed design finds its knee near 262 req/s (2.62x the profile's rates), to the sweep's search resolution: the last point within the plain limit (no margin) before the first one over it.

## Re-run

```bash
kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --seed 7
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
      "cost_micro_usd_hr": 1634500,
      "rejudged": false
    },
    "end": {
      "capacity_req_s": 251.0,
      "error_rate": 0.0,
      "p99_ms": 300.0
    },
    "families": [
      {
        "flavor": "dynamodb",
        "kind": "answer",
        "node": "db",
        "notes": [
          "the table is priced as provisioned throughput",
          "assumes your application can use this family; preset estimates"
        ],
        "plan": {
          "end": {
            "capacity_req_s": 300.0,
            "error_rate": 0.0,
            "p99_ms": 280.0
          },
          "families": [],
          "price_after": 1200000,
          "price_before": 700000,
          "start": {
            "capacity_req_s": 150.0,
            "error_rate": 0.01,
            "p99_ms": 1500.0
          },
          "steps": [
            {
              "after": {
                "capacity_req_s": 300.0,
                "error_rate": 0.0,
                "p99_ms": 280.0
              },
              "before": {
                "capacity_req_s": 150.0,
                "error_rate": 0.01,
                "p99_ms": 1500.0
              },
              "menu": [],
              "mv": {
                "kind": {
                  "from": 2,
                  "knob": "copies",
                  "to": 3
                },
                "node": "app"
              },
              "note": null,
              "price_delta_micro_usd_hr": 500000,
              "price_micro_usd_hr": 1200000
            }
          ],
          "unpriced_nodes": [],
          "verdict": {
            "kind": "holds"
          }
        }
      },
      {
        "flavor": "mongodb",
        "kind": "skipped",
        "node": "db",
        "reason": "no preset instance table for this flavor"
      }
    ],
    "label": "relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro_usd are millionths of a dollar), of the priced nodes only",
    "measured_knee": {
      "k": 2.62,
      "kind": "knee",
      "req_s": 262.0
    },
    "price_after": 1620000,
    "price_before": 840000,
    "request": {
      "budget_micro_usd_hr": 2000000,
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
    "start": {
      "capacity_req_s": 49.0,
      "error_rate": 0.05,
      "p99_ms": 4100.0
    },
    "steps": [
      {
        "after": {
          "capacity_req_s": 118.0,
          "error_rate": 0.002,
          "p99_ms": 1200.0
        },
        "before": {
          "capacity_req_s": 49.0,
          "error_rate": 0.05,
          "p99_ms": 4100.0
        },
        "menu": [
          {
            "effect": {
              "capacity_req_s": 83.0,
              "error_rate": 0.015,
              "p99_ms": 2900.0
            },
            "holds": false,
            "mv": {
              "kind": {
                "from": "db.m5.large",
                "knob": "class",
                "to": "db.m5.xlarge"
              },
              "node": "db"
            },
            "price_micro_usd_hr": 1130000
          },
          {
            "effect": {
              "capacity_req_s": 118.0,
              "error_rate": 0.002,
              "p99_ms": 1200.0
            },
            "holds": false,
            "mv": {
              "kind": {
                "from": "db.m5.large",
                "knob": "class",
                "to": "db.m5.2xlarge"
              },
              "node": "db"
            },
            "price_micro_usd_hr": 1420000
          },
          {
            "effect": {
              "capacity_req_s": 77.0,
              "error_rate": 0.02,
              "p99_ms": 3300.0
            },
            "holds": false,
            "mv": {
              "kind": {
                "from": 1,
                "knob": "copies",
                "to": 2
              },
              "node": "db"
            },
            "price_micro_usd_hr": 1090000
          }
        ],
        "mv": {
          "kind": {
            "from": "db.m5.large",
            "knob": "class",
            "to": "db.m5.2xlarge"
          },
          "node": "db"
        },
        "note": null,
        "price_delta_micro_usd_hr": 580000,
        "price_micro_usd_hr": 1420000
      },
      {
        "after": {
          "capacity_req_s": 251.0,
          "error_rate": 0.0,
          "p99_ms": 300.0
        },
        "before": {
          "capacity_req_s": 118.0,
          "error_rate": 0.002,
          "p99_ms": 1200.0
        },
        "menu": [
          {
            "effect": {
              "capacity_req_s": 251.0,
              "error_rate": 0.0,
              "p99_ms": 300.0
            },
            "holds": true,
            "mv": {
              "kind": {
                "from": 2,
                "knob": "copies",
                "to": 3
              },
              "node": "app"
            },
            "price_micro_usd_hr": 1620000
          },
          {
            "effect": {
              "capacity_req_s": 160.0,
              "error_rate": 0.001,
              "p99_ms": 800.0
            },
            "holds": true,
            "mv": {
              "kind": {
                "from": "c5.large",
                "knob": "class",
                "to": "c5.xlarge"
              },
              "node": "app"
            },
            "price_micro_usd_hr": 1700000
          }
        ],
        "mv": {
          "kind": {
            "from": 2,
            "knob": "copies",
            "to": 3
          },
          "node": "app"
        },
        "note": "a second copy of app halves its queue",
        "price_delta_micro_usd_hr": 200000,
        "price_micro_usd_hr": 1620000
      }
    ],
    "unpriced_nodes": [
      "cdn"
    ],
    "verdict": {
      "kind": "holds"
    },
    "verify": {
      "capacity_req_s": 251.0,
      "cost": {
        "egress": [],
        "nodes": [],
        "priced": 3,
        "regions": {},
        "total_micro_usd_hr": 1634500,
        "unpriced": 1
      },
      "error_rate": 0.0,
      "holds": true,
      "p99_ms": 300.0
    }
  },
  "recipe": {
    "chaos": "none",
    "chaos_digest": "9f86d081884c",
    "command": "kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --seed 7",
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
