# Load sweep s3

*relative what-if, not a prediction*

## Finding

**Verdict:** knee at 200 req/s (2.00x): p99 over 1s and errors over 1%.

- Limit: p99 1s, errors 1%.
- First to saturate: db at 400 req/s (4.00x), analytic 400 req/s.
- Dominant cause: overflow.

Warnings:

- at 4.000x the p99 rests on 900 requests

## Curve

| load req/s | goodput | p99 ms | errors | ok |
| ---: | ---: | ---: | ---: | --- |
| 100.0 | 100.0 | 20.0 | 0% | yes |
| 200.0 | 199.5 | 250.0 | 0.25% | yes |
| 400.0 | 380.0 | 2500.0 | 5% | no (p99, errors) |

## Re-run

```bash
kaanal sim sweep --profile=peak --p99 1s --errors 1% --max-factor 8 --seed 42
# seed 42: every point draws the same random numbers, so the same seed on the same design repeats the curve
# fingerprint 0123456789ab: SHA-256 of every file the build read, comments and whitespace aside; the same value means the same design, so the same numbers
# kaanal 9.9.9
```

## Data

```json
{
  "finding": {
    "dominant_cause": "overflow",
    "first_to_saturate": {
      "analytic_req_s": 400.0,
      "k": 4.0,
      "node": "db",
      "req_s": 400.0
    },
    "limit": {
      "error_rate": 0.01,
      "p99_ms": 1000.0
    },
    "recovers_at": null,
    "stopped_at": null,
    "verdict": {
      "k": 2.0,
      "kind": "knee",
      "req_s": 200.0
    },
    "warnings": [
      "at 4.000x the p99 rests on 900 requests"
    ]
  },
  "id": "s3",
  "label": "relative what-if, not a prediction",
  "points": [
    {
      "cause": null,
      "error_rate": 0.0,
      "goodput_req_s": 100.0,
      "k": 1.0,
      "over": {
        "errors": false,
        "p99": false
      },
      "p50_ms": 5.0,
      "p95_ms": 10.0,
      "p99_ms": 20.0,
      "req_s": 100.0,
      "requests": 20000
    },
    {
      "cause": null,
      "error_rate": 0.0025,
      "goodput_req_s": 199.5,
      "k": 2.0,
      "over": {
        "errors": false,
        "p99": false
      },
      "p50_ms": 62.5,
      "p95_ms": 125.0,
      "p99_ms": 250.0,
      "req_s": 200.0,
      "requests": 20000
    },
    {
      "cause": "overflow",
      "error_rate": 0.05,
      "goodput_req_s": 380.0,
      "k": 4.0,
      "over": {
        "errors": true,
        "p99": true
      },
      "p50_ms": 625.0,
      "p95_ms": 1250.0,
      "p99_ms": 2500.0,
      "req_s": 400.0,
      "requests": 900
    }
  ],
  "recipe": {
    "command": "kaanal sim sweep --profile=peak --p99 1s --errors 1% --max-factor 8 --seed 42",
    "fingerprint": "0123456789ab",
    "generation": 2,
    "inline_profile": null,
    "kaanal_version": "9.9.9",
    "max_factor": 8.0,
    "profile": "peak",
    "scenario": null,
    "seed": 42
  },
  "report_version": 1
}
```
