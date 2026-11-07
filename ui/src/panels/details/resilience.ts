// What a wire does when a call over it fails (step 7, RULES 82-91), as data: where each setting lives
// in the config and the patch that edits it, as `edgeFields.ts` does for the wire's cost. Retries,
// the retry budget, `abandon`, the circuit breaker and `detect` are all off until written, so a box
// shows the loader's default and "off" removes the key. The loader's limits are in the constants; a
// value outside them would make the draft invalid, so the fields never send one.

import type { EdgeDoc, NodeDoc, Op } from "../../data/api/types";
import { durationMs, formatMs } from "./fields";

type Obj = Record<string, unknown>;
const isObject = (v: unknown): v is Obj => v !== null && typeof v === "object" && !Array.isArray(v);

const set = (path: (string | number)[], present: boolean, value: unknown): Op => ({
  op: present ? "set" : "insert",
  path,
  value,
});

/** Where a thing with a `retry:` sits: an edge by index, or any path (a `call:` step of a route's chain). */
export type At = number | (string | number)[];
const base = (i: At): (string | number)[] => (typeof i === "number" ? ["edges", i] : i);

// ---- retry (RULES 82-84) ----

/** The loader's limits and defaults for `retry` (`config/retry.rs`). */
export const RETRY = {
  attempts: { min: 2, max: 5, default: 3 },
  backoffMs: { min: 1, max: 3_600_000, default: 50 },
  multiplier: { min: 1, max: 10, default: 2 },
  capMs: { min: 1, max: 3_600_000, default: 20_000 },
  budgetTokens: { min: 1, max: 1000, default: 10 },
  /** Tokens a success adds back, above 0 and at most 1. */
  budgetRatio: { min: 0.001, max: 1, default: 0.1 },
} as const;

export type Jitter = "full" | "equal" | "none";
export const JITTERS: readonly { value: Jitter; label: string }[] = [
  { value: "full", label: "Full: anywhere from 0 to the wait (default)" },
  { value: "equal", label: "Equal: half the wait, plus up to half again" },
  { value: "none", label: "None: always the whole wait" },
];

/** The edge's `retry` when it has one (an empty `{}` is the defaults); null = no retry. */
export const retryOf = (e: EdgeDoc): Obj | null => (isObject(e.retry) ? e.retry : null);

/** Retry on takes the defaults (`{}`); off removes the key. */
export const setRetryOn = (i: number, e: EdgeDoc, on: boolean): Op[] =>
  on
    ? "retry" in e
      ? []
      : [set(["edges", i, "retry"], false, {})]
    : "retry" in e
      ? [{ op: "remove", path: ["edges", i, "retry"] }]
      : [];

/** Sets one key of `retry` (the box must only exist while `retry` does). */
export const setRetry = (i: At, r: Obj, key: string, value: unknown): Op =>
  set([...base(i), "retry", key], key in r, value);
export const clearRetry = (i: At, r: Obj, key: string): Op[] =>
  key in r ? [{ op: "remove", path: [...base(i), "retry", key] }] : [];

export const attemptsOf = (r: Obj): number =>
  typeof r.max_attempts === "number" ? r.max_attempts : RETRY.attempts.default;
export const multiplierOf = (r: Obj): number =>
  typeof r.multiplier === "number" ? r.multiplier : RETRY.multiplier.default;
export const backoffMsOf = (r: Obj): number | null => durationMs(r.backoff);
export const capMsOf = (r: Obj): number | null => durationMs(r.cap);
export const setRetryMs = (i: At, r: Obj, key: "backoff" | "cap", ms: number): Op =>
  setRetry(i, r, key, formatMs(ms));
export const jitterOf = (r: Obj): Jitter =>
  r.jitter === "equal" || r.jitter === "none" ? r.jitter : "full";
/** The default (`full`) removes the key. */
export const setJitter = (i: At, r: Obj, j: Jitter): Op[] =>
  j === "full" ? clearRetry(i, r, "jitter") : [setRetry(i, r, "jitter", j)];
export const writesOf = (r: Obj): boolean => r.writes === true;
/** Off (the default) removes the key. */
export const setWrites = (i: At, r: Obj, on: boolean): Op[] =>
  on ? [setRetry(i, r, "writes", true)] : clearRetry(i, r, "writes");

/** The retry budget (gRPC A6) when written; `{}` is 10 tokens refilling 0.1 a success. */
export const budgetOf = (r: Obj): Obj | null => (isObject(r.budget) ? r.budget : null);
export const setBudgetOn = (i: At, r: Obj, on: boolean): Op[] =>
  on ? ("budget" in r ? [] : [setRetry(i, r, "budget", {})]) : clearRetry(i, r, "budget");
export const budgetTokensOf = (b: Obj): number =>
  typeof b.max_tokens === "number" ? b.max_tokens : RETRY.budgetTokens.default;
export const budgetRatioOf = (b: Obj): number =>
  typeof b.token_ratio === "number" ? b.token_ratio : RETRY.budgetRatio.default;
export const setBudget = (i: At, b: Obj, key: "max_tokens" | "token_ratio", v: number): Op =>
  set([...base(i), "retry", "budget", key], key in b, v);

// ---- abandon (RULES 88) ----

export type AbandonChoice = "cancel" | "continue";
export const ABANDON_CHOICES: readonly { value: AbandonChoice; label: string }[] = [
  { value: "cancel", label: "Stop the work (default)" },
  { value: "continue", label: "Let the callee finish it" },
];
export const abandonOf = (e: EdgeDoc): AbandonChoice =>
  e.abandon === "continue" ? "continue" : "cancel";
/** The default removes the key. */
export const setAbandon = (i: number, e: EdgeDoc, c: AbandonChoice): Op[] =>
  c === "cancel"
    ? "abandon" in e
      ? [{ op: "remove", path: ["edges", i, "abandon"] }]
      : []
    : [set(["edges", i, "abandon"], "abandon" in e, c)];

// ---- circuit breaker (RULES 90) ----

/** The loader's limits and defaults for `breaker` (`config/breaker.rs`, resilience4j's values). */
export const BREAKER = {
  window: { min: 1, max: 1000, default: 100 },
  minCalls: { min: 1, max: 65_535, default: 100 },
  /** Percent; the config's 0.001 to 1, times 100. */
  ratePct: { min: 0.1, max: 100, default: 50 },
  openForMs: { min: 1, max: 3_600_000, default: 60_000 },
  probes: { min: 1, max: 100, default: 10 },
} as const;

export const breakerOf = (e: EdgeDoc): Obj | null => (isObject(e.breaker) ? e.breaker : null);
export const setBreakerOn = (i: number, e: EdgeDoc, on: boolean): Op[] =>
  on
    ? "breaker" in e
      ? []
      : [set(["edges", i, "breaker"], false, {})]
    : "breaker" in e
      ? [{ op: "remove", path: ["edges", i, "breaker"] }]
      : [];
export const setBreaker = (i: number, b: Obj, key: string, value: unknown): Op =>
  set(["edges", i, "breaker", key], key in b, value);
export const clearBreaker = (i: number, b: Obj, key: string): Op[] =>
  key in b ? [{ op: "remove", path: ["edges", i, "breaker", key] }] : [];

export const windowOf = (b: Obj): number =>
  typeof b.window === "number" ? b.window : BREAKER.window.default;
export const minCallsOf = (b: Obj): number =>
  typeof b.min_calls === "number" ? b.min_calls : BREAKER.minCalls.default;
export const probesOf = (b: Obj): number =>
  typeof b.half_open_calls === "number" ? b.half_open_calls : BREAKER.probes.default;
/** The failure rate in percent. */
export const ratePctOf = (b: Obj): number =>
  typeof b.failure_rate === "number"
    ? Number((b.failure_rate * 100).toFixed(4))
    : BREAKER.ratePct.default;
export const setRatePct = (i: number, b: Obj, pct: number): Op =>
  setBreaker(i, b, "failure_rate", Number((pct / 100).toFixed(6)));
export const openForMsOf = (b: Obj): number | null =>
  "open_for" in b ? durationMs(b.open_for) : BREAKER.openForMs.default;
export const setOpenFor = (i: number, b: Obj, ms: number): Op =>
  setBreaker(i, b, "open_for", formatMs(ms));

// ---- detect (RULES 91) ----

/** The loader's cap on `detect`: one hour (0 is allowed: notice at once). */
export const DETECT_MAX_MS = 3_600_000;
/** An edge's or a balancer's `detect` in ms; null when it is not written (zero applies). */
export const detectMsOf = (v: Record<string, unknown>): number | null => durationMs(v.detect);
export const setDetectEdge = (i: number, e: EdgeDoc, ms: number): Op =>
  set(["edges", i, "detect"], "detect" in e, formatMs(ms));
export const clearDetectEdge = (i: number, e: EdgeDoc): Op[] =>
  "detect" in e ? [{ op: "remove", path: ["edges", i, "detect"] }] : [];
export const setDetectNode = (name: string, n: NodeDoc, ms: number): Op =>
  set(["nodes", name, "detect"], "detect" in n, formatMs(ms));
export const clearDetectNode = (name: string, n: NodeDoc): Op[] =>
  "detect" in n ? [{ op: "remove", path: ["nodes", name, "detect"] }] : [];
