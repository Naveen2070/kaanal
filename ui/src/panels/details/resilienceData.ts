// What the Results charts for retries and breakers draw, and what the wire panel's effect strip says,
// worked out from the run summary's `retries` and `breakers` lists (step 7f). Pure, so each is tested
// without drawing. Nothing is estimated: every figure is a count the engine made.

import type { BreakerPhase, BreakerSummary, RetrySummary } from "../../data/api/types";

export interface FunnelSegment {
  key: "first" | "recovered" | "gave_up" | "failed";
  label: string;
  count: number;
}

/**
 * How the calls over one wire ended: went through first time, recovered by a retry, retried and gave
 * up, or failed with no retry (not transient, or no budget). The four add up to `calls`; a call still
 * waiting out a backoff when its request was answered is in none, so the first segment is clamped.
 */
export function funnelOf(r: RetrySummary): FunnelSegment[] {
  const failedWithoutRetry = Math.max(0, r.failed - r.gave_up);
  const first = Math.max(0, r.calls - r.recovered - r.gave_up - failedWithoutRetry);
  return [
    { key: "first", label: "Went through first time", count: first },
    { key: "recovered", label: "Recovered by a retry", count: r.recovered },
    { key: "gave_up", label: "Retried, then gave up", count: r.gave_up },
    { key: "failed", label: "Failed without a retry", count: failedWithoutRetry },
  ];
}

export interface WireEffects {
  /** Calls whose last attempt failed, as a share of calls (0 to 100). */
  failedPct: number;
  /** Retried calls that ended well, as a share of retried calls; null when none was retried. */
  recoveredPct: number | null;
  /** Retry attempts per call. */
  retriesPerCall: number;
  /** The share of the window the wire's breaker was open; null when it has none. */
  openPct: number | null;
}

/** The figures of the wire panel's effect strip; null when nothing carried a retry scope. */
export function wireEffects(
  r: RetrySummary,
  b: BreakerSummary | undefined,
  windowMs: number,
): WireEffects | null {
  if (!(r.calls > 0)) return null;
  return {
    failedPct: Math.round((r.failed / r.calls) * 100),
    recoveredPct: r.retried > 0 ? Math.round((r.recovered / r.retried) * 100) : null,
    retriesPerCall: Number((r.retries / r.calls).toFixed(1)),
    openPct: b && windowMs > 0 ? Math.round((b.open_ms / windowMs) * 100) : null,
  };
}

export interface Band {
  state: BreakerPhase;
  /** Milliseconds from the start of the window. */
  from: number;
  to: number;
}

/** The breaker's state over the window as consecutive bands; changes past the window are dropped. */
export function bandsOf(b: BreakerSummary, windowMs: number): Band[] {
  const bands: Band[] = [];
  let state = b.state_at_start;
  let from = 0;
  for (const c of b.changes) {
    if (c.t_ms >= windowMs) break;
    if (c.t_ms > from) bands.push({ state, from, to: c.t_ms });
    state = c.state;
    from = c.t_ms;
  }
  if (windowMs > from) bands.push({ state, from, to: windowMs });
  return bands;
}
