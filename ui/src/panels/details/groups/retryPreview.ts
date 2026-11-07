// Two pure helpers behind the "When a call fails" group: the waits a retry policy produces (for the
// backoff picture) and the rows that differ from the default (for the compare table). Numbers come
// from the same loader constants the boxes use (`resilience.ts`), never copied.
//
// Waits: exponential backoff as gRPC A6 and the AWS Architecture Blog ("Exponential Backoff and
// Jitter") state it: the wait before retry n is min(cap, initial * multiplier^(n-1)). Jitter is not
// applied here: the picture shows the longest a wait can be.

import type { EdgeDoc } from "../../../data/api/types";
import {
  abandonOf,
  attemptsOf,
  backoffMsOf,
  budgetOf,
  capMsOf,
  jitterOf,
  multiplierOf,
  RETRY,
  retryOf,
  writesOf,
} from "../resilience";
import { humanMs } from "./summaries";

type Obj = Record<string, unknown>;

/** The wait before each retry, in ms: `max_attempts - 1` of them, each capped at the longest wait. */
export function retryWaits(r: Obj): number[] {
  const first = backoffMsOf(r) ?? RETRY.backoffMs.default;
  const growth = multiplierOf(r);
  const cap = capMsOf(r) ?? RETRY.capMs.default;
  return Array.from({ length: attemptsOf(r) - 1 }, (_, k) => Math.min(cap, first * growth ** k));
}

export interface DiffRow {
  label: string;
  was: string;
  now: string;
}

/** Only the settings of the retry group that differ from the loader's defaults, in the panel's order. */
export function retryDiff(e: EdgeDoc): DiffRow[] {
  const rows: DiffRow[] = [];
  const add = (label: string, was: string, now: string) => {
    if (was !== now) rows.push({ label, was, now });
  };
  const r = retryOf(e);
  add("Retry", "off", r ? "on" : "off");
  if (r) {
    add("Tries", String(RETRY.attempts.default), String(attemptsOf(r)));
    add(
      "First wait",
      humanMs(RETRY.backoffMs.default),
      humanMs(backoffMsOf(r) ?? RETRY.backoffMs.default),
    );
    add("Growth of the wait", `×${RETRY.multiplier.default}`, `×${multiplierOf(r)}`);
    add("Longest wait", humanMs(RETRY.capMs.default), humanMs(capMsOf(r) ?? RETRY.capMs.default));
    add("Randomise the wait", "full", jitterOf(r));
    add("Retry writes", "off", writesOf(r) ? "on" : "off");
    add("Budget", "off", budgetOf(r) ? "on" : "off");
  }
  add(
    "When the caller gives up",
    "stops the work",
    abandonOf(e) === "continue" ? "work continues" : "stops the work",
  );
  return rows;
}
