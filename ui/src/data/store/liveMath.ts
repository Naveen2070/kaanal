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

/** Which requests a figure counts: all of them, the real (frontend) ones, or the generated ones. */
export type Source = "all" | "real" | "generated";

/** Like `sumHist`, for one source; generated is all minus real. */
export function sumHistOf(ticks: readonly Tick[], src: Source): Map<number, number> {
  if (src === "all") return sumHist(ticks);
  const real = new Map<number, number>();
  for (const t of ticks)
    for (const [b, c] of t.real?.hist ?? []) real.set(b, (real.get(b) ?? 0) + c);
  if (src === "real") return real;
  const out = sumHist(ticks);
  for (const [b, c] of real) {
    const left = (out.get(b) ?? 0) - c;
    if (left > 0) out.set(b, left);
    else out.delete(b);
  }
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

export function windowFigures(
  ticks: readonly Tick[],
  ms: number,
  src: Source = "all",
): WindowFigures {
  const w = recent(ticks, ms);
  const seconds = Math.max(w.length * 0.25, 0.25);
  const count = (pick: (t: Tick) => number, realPick: (t: Tick) => number) =>
    w.reduce(
      (a, t) =>
        a + (src === "all" ? pick(t) : src === "real" ? realPick(t) : pick(t) - realPick(t)),
      0,
    );
  const requests = count(
    (t) => t.requests,
    (t) => t.real?.requests ?? 0,
  );
  const failed = count(
    (t) => t.failed,
    (t) => t.real?.failed ?? 0,
  );
  const hist = sumHistOf(w, src);
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
 * Finished points by the first tick object of their chunk: a full chunk never changes, so it is
 * summed once. A hit needs the same chunk size and the same last tick object, so a restarted
 * session (new tick objects) can never read an old point.
 */
const chunkCache = new WeakMap<Tick, { per: number; last: Tick | undefined; point: Point }>();

/**
 * The window as at most `points` points (a 30 minute window is 7,200 ticks): consecutive ticks are
 * added up, so a point's rates and its p99 are those of the span it stands for. A full chunk is
 * kept by its first tick, so a tick that arrives re-sums only the chunk it lands in, not the window.
 */
export function timelineSeries(ticks: readonly Tick[], points = 240): Point[] {
  if (ticks.length === 0) return [];
  const per = Math.max(1, Math.ceil(ticks.length / points));
  const out: Point[] = [];
  for (let i = 0; i < ticks.length; i += per) {
    const full = i + per <= ticks.length;
    const first = ticks[i];
    const last = ticks[i + per - 1];
    const hit = full && first ? chunkCache.get(first) : undefined;
    if (hit && hit.per === per && hit.last === last) {
      out.push(hit.point);
      continue;
    }
    const chunk = ticks.slice(i, i + per);
    const seconds = chunk.length * 0.25;
    const point: Point = {
      t_ms: chunk.at(-1)?.t_ms ?? 0,
      reqPerS: chunk.reduce((a, t) => a + t.requests, 0) / seconds,
      droppedPerS: chunk.reduce((a, t) => a + t.failed, 0) / seconds,
      p99Ms: quantileMs(sumHist(chunk), 0.99),
    };
    if (full && first) chunkCache.set(first, { per, last, point });
    out.push(point);
  }
  return out;
}
