// Reading the live ticks: the latency histogram's buckets and the figures of a window. The bucket
// layout mirrors `kaanal-server/src/live/hist.rs` (geometric, ratio 1.0201, 10 microseconds up),
// so a quantile read here is within about 1% of the exact one. Everything is a sum over ticks.

import type { Tick } from "../api/liveTypes";

const MIN_NS = 10_000;
const RATIO = 1.0201;

/** The latency (ns) a bucket stands for: the geometric middle of its range. */
export function bucketNs(i: number): number {
  if (i <= 0) return MIN_NS / 2;
  return Math.floor(MIN_NS * RATIO ** (i - 0.5));
}

/** The bucket of a latency in ns (the inverse of `bucketNs` to within a bucket). */
export function bucketOf(ns: number): number {
  if (ns < MIN_NS) return 0;
  return 1 + Math.floor(Math.log(ns / MIN_NS) / Math.log(RATIO));
}

/** Adds the histograms of `ticks` into one `bucket -> count` map. */
export function sumHist(ticks: readonly Tick[]): Map<number, number> {
  const out = new Map<number, number>();
  for (const t of ticks) for (const [b, c] of t.hist) out.set(b, (out.get(b) ?? 0) + c);
  return out;
}

/** The latency in ms below which a share `q` of the counted requests fall; null with none. */
export function quantileMs(hist: ReadonlyMap<number, number>, q: number): number | null {
  let total = 0;
  for (const c of hist.values()) total += c;
  if (total === 0) return null;
  const rank = Math.max(1, Math.ceil(Math.min(1, Math.max(0, q)) * total));
  let seen = 0;
  for (const b of [...hist.keys()].sort((a, c) => a - c)) {
    seen += hist.get(b) ?? 0;
    if (seen >= rank) return bucketNs(b) / 1e6;
  }
  return null;
}

/** The ticks that ended in the last `ms` milliseconds of the newest one. */
export function recent(ticks: readonly Tick[], ms: number): Tick[] {
  const last = ticks.at(-1);
  if (!last) return [];
  // walk back from the newest: a short span of a 7,200 tick window costs only its own ticks
  let i = ticks.length;
  while (i > 0 && (ticks[i - 1] as Tick).t_ms > last.t_ms - ms) i--;
  return ticks.slice(i);
}

/** The strip's figures over the last `ms` of ticks: per second, and the percentiles. */
export interface WindowFigures {
  /** Requests finished a second (served and dropped). */
  reqPerS: number;
  /** Dropped a second. */
  droppedPerS: number;
  p50Ms: number | null;
  p99Ms: number | null;
}

export function windowFigures(ticks: readonly Tick[], ms: number): WindowFigures {
  const w = recent(ticks, ms);
  const seconds = Math.max(w.length * 0.25, 0.25);
  const requests = w.reduce((a, t) => a + t.requests, 0);
  const failed = w.reduce((a, t) => a + t.failed, 0);
  const hist = sumHist(w);
  return {
    reqPerS: requests / seconds,
    droppedPerS: failed / seconds,
    p50Ms: quantileMs(hist, 0.5),
    p99Ms: quantileMs(hist, 0.99),
  };
}

/** One point of the timeline: a few ticks added up. */
export interface Point {
  /** Milliseconds since the session started, at the end of the point. */
  t_ms: number;
  reqPerS: number;
  droppedPerS: number;
  p99Ms: number | null;
}

/**
 * The window as at most `points` points (a 30 minute window is 7,200 ticks): consecutive ticks are
 * added up, so a point's rates and its p99 are those of the span it stands for.
 */
export function timelineSeries(ticks: readonly Tick[], points = 240): Point[] {
  if (ticks.length === 0) return [];
  const per = Math.max(1, Math.ceil(ticks.length / points));
  const out: Point[] = [];
  for (let i = 0; i < ticks.length; i += per) {
    const chunk = ticks.slice(i, i + per);
    const seconds = chunk.length * 0.25;
    out.push({
      t_ms: chunk.at(-1)?.t_ms ?? 0,
      reqPerS: chunk.reduce((a, t) => a + t.requests, 0) / seconds,
      droppedPerS: chunk.reduce((a, t) => a + t.failed, 0) / seconds,
      p99Ms: quantileMs(sumHist(chunk), 0.99),
    });
  }
  return out;
}
