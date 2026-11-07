// Help for the chart titles of the Results tab. Same shape and rules as `copyWire.ts`: the help popover
// says `what`, `example` and `hint`, and the glossary chapter "Reading Results" is built from these
// same objects (`contentResults.ts`), so the two never differ. Examples are worked from the way the
// charts are drawn, not from a real system's numbers.

import type { Help } from "./copy";

const DEFS = {
  "latency-map": {
    label: "Latency heat map",
    what: "Each column is a slice of the run, each row a latency band. The brighter the cell, the more requests landed there.",
    example:
      "A bright band that climbs from the bottom toward the top as the run goes on means requests are getting slower: a queue is building.",
    hint: "Pick another component or wire in the box above the chart to compare where the time goes.",
  },
  "busy-run": {
    label: "Busy over the run",
    what: "The share of this component's slots in use in each slice of the run. At 100% every slot is taken.",
    example:
      "A line that sits at 100% from the 10 s mark on means this component is the limit from then.",
    hint: "The gap under the line is spare capacity; a line pressed against 100% is saturated.",
  },
  funnel: {
    label: "How calls ended",
    what: "Calls over a retrying wire end four ways: first try, a retry that worked, retries that gave up, or failure with no retry.",
    example:
      "Of 100 calls: 55 first time, 25 recovered by a retry, 15 retried and gave up, 5 failed with no retry.",
    hint: 'A big "gave up" slice means retries are not helping: try a retry budget or a circuit breaker.',
  },
  "breaker-bands": {
    label: "Circuit breakers",
    what: "Closed lets calls through, open refuses them at once, half open lets probes decide. The bands show which it was.",
    example:
      "Open from 3 s to 13 s means every call in that time got an instant 503 and the callee had room to recover.",
    hint: "A breaker that opens, probes and opens again is protecting a callee that is still failing.",
  },
  "errors-kind": {
    label: "Errors by kind",
    what: "Failed requests counted by how they failed: status (an error reply), timeout, reset or malformed.",
    example: "timeout: 700 means 700 requests waited past their deadline.",
    hint: "Mostly timeouts point at a slow or full component; mostly status errors at one that is failing.",
  },
} satisfies Record<string, Help>;

/** Typed by id, with every field but `label` and `what` optional (a literal would narrow each entry). */
export const RESULTS_HELP: Record<keyof typeof DEFS, Help> = DEFS;
