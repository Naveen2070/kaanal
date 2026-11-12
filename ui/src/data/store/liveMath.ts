// Reading the live ticks: the latency histogram's buckets and the figures of a window. The bucket
// layout mirrors `kaanal-server/src/live/hist.rs` (geometric, ratio 1.0201, 10 microseconds up),
// so a quantile read here is within about 1% of the exact one. Everything is a sum over ticks.

import type { BreakerTick, Tick } from "../api/liveTypes";

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

/** The live window as the charts band reads it: one value per point, as rates a second. */
export interface LiveSeries {
  /** Seconds one point stands for. */
  window_s: number;
  p99_ms: number[];
  /** Served a second (requests that finished minus the failed). */
  done: number[];
  failed: number[];
}

export function liveSeries(ticks: readonly Tick[]): LiveSeries | undefined {
  const pts = timelineSeries(ticks);
  if (pts.length < 2) return undefined;
  return {
    window_s: (ticks.length * 0.25) / pts.length,
    p99_ms: pts.map((p) => p.p99Ms ?? 0),
    done: pts.map((p) => Math.max(0, p.reqPerS - p.droppedPerS)),
    failed: pts.map((p) => p.droppedPerS),
  };
}

export const KIND_LABEL: Record<string, string> = {
  overflow: "queue full",
  rate_limit: "rate limited",
  deadline: "timed out",
  breaker: "breaker open",
  oom: "out of memory",
  chaos: "chaos",
  pinned: "pinned error",
};

/** Failures over the window by cause, most first; the node drops of every tick added up. */
export function errorKinds(ticks: readonly Tick[]): [string, number][] {
  // fixed keys, no per-tick object walk: this runs on every tick over a window of up to 7,200
  const keys = Object.keys(KIND_LABEL) as (keyof Tick["nodes"][number]["dropped"])[];
  const total = Array.from({ length: keys.length }, () => 0);
  for (const t of ticks)
    for (const n of t.nodes)
      keys.forEach((k, i) => (total[i] = (total[i] ?? 0) + (n.dropped[k] ?? 0)));
  return keys
    .map((k, i): [string, number] => [KIND_LABEL[k] ?? k, total[i] ?? 0])
    .filter(([, v]) => v > 0)
    .sort((a, b) => b[1] - a[1]);
}

/** How a wire did over the last `ms`: crossings a second, the mean time on it and the mean wait to get on. */
export interface WireFigures {
  perS: number;
  meanWireMs: number | null;
  meanWaitMs: number | null;
  /** Calls over the wire that were first tries, and the shares that were retried and retried again. */
  retriedPct: number | null;
  retriesPerCall: number | null;
  /** The engine's own retry counts over the window: calls, those retried, and how they ended. */
  funnel: { calls: number; retried: number; recovered: number; gaveUp: number } | null;
  /** How full the wire was over the window (0 to 1), for a wire with a `bandwidth`; null otherwise. */
  load: number | null;
}

export function wireFigures(ticks: readonly Tick[], edge: number, ms = 5000): WireFigures {
  const w = recent(ticks, ms);
  let carried = 0;
  let wire = 0;
  let wait = 0;
  let retries = 0;
  let retried = 0;
  const fn = { calls: 0, retried: 0, recovered: 0 };
  let loadSum = 0;
  let loadN = 0;
  for (const t of w) {
    const x = t.wires[edge];
    if (!x) continue;
    if (x.funnel) {
      fn.calls += x.funnel.calls;
      fn.retried += x.funnel.retried;
      fn.recovered += x.funnel.recovered;
    }
    if (typeof x.load === "number") {
      loadSum += x.load;
      loadN++;
    }
    carried += x.carried;
    wire += x.wire_ns ?? 0;
    wait += x.wait_ns ?? 0;
    retries += x.retries ?? 0;
    retried += x.retried ?? 0;
  }
  // a retry runs where the first try did, so it is not a call of its own: calls = carried - retries
  const calls = carried - retries;
  return {
    perS: carried / Math.max(w.length * 0.25, 0.25),
    meanWireMs: carried > 0 ? wire / carried / 1e6 : null,
    meanWaitMs: carried > 0 ? wait / carried / 1e6 : null,
    retriedPct: retries > 0 && calls > 0 ? (retried / calls) * 100 : null,
    retriesPerCall: retries > 0 && calls > 0 ? retries / calls : null,
    // retried minus recovered is the calls that gave up, plus the few still retrying
    funnel: fn.retried > 0 ? { ...fn, gaveUp: Math.max(0, fn.retried - fn.recovered) } : null,
    load: loadN > 0 ? Math.min(1, loadSum / loadN) : null,
  };
}

/** What one route of the offered mix did over the last `ms`. */
export interface EndpointFigures {
  reqPerS: number;
  failedPerS: number;
  p50Ms: number | null;
  p99Ms: number | null;
}

export function endpointFigures(
  ticks: readonly Tick[],
  stream: number,
  ms = 10_000,
): EndpointFigures {
  const w = recent(ticks, ms);
  const seconds = Math.max(w.length * 0.25, 0.25);
  let requests = 0;
  let failed = 0;
  const hist = new Map<number, number>();
  for (const t of w) {
    const e = t.endpoints?.[stream];
    if (!e) continue;
    requests += e.requests;
    failed += e.failed;
    for (const [b, c] of e.hist) hist.set(b, (hist.get(b) ?? 0) + c);
  }
  return {
    reqPerS: requests / seconds,
    failedPerS: failed / seconds,
    p50Ms: quantileMs(hist, 0.5),
    p99Ms: quantileMs(hist, 0.99),
  };
}

export type BreakerPhase = BreakerTick["state"];

/** One breaker over the window: its state at the end of each tick, oldest first. */
export interface BreakerBand {
  replica: number;
  phases: BreakerPhase[];
  /** Seconds it was open (or half open) in the window, and the state now. */
  openS: number;
  now: BreakerPhase;
}

/** The breakers of wire `edge` over the last `ms`, from the state each tick reported. */
export function breakerBands(ticks: readonly Tick[], edge: number, ms = 60_000): BreakerBand[] {
  const w = recent(ticks, ms);
  const replicas = new Set<number>();
  for (const t of w) for (const b of t.breakers ?? []) if (b.edge === edge) replicas.add(b.replica);
  return [...replicas]
    .sort((a, b) => a - b)
    .map((replica) => {
      const phases = w.map(
        (t): BreakerPhase =>
          t.breakers?.find((b) => b.edge === edge && b.replica === replica)?.state ?? "closed",
      );
      return {
        replica,
        phases,
        openS: phases.filter((p) => p !== "closed").length * 0.25,
        now: phases.at(-1) ?? "closed",
      };
    });
}

/** The heatmap's rows for the live window: `<= 0.25 ms`, doubling up to about 33 s, then "slower". */
export const LIVE_BUCKETS: { le_ms: number | null; count: number }[] = [
  ...Array.from({ length: 18 }, (_, k) => ({ le_ms: 0.25 * 2 ** k, count: 0 })),
  { le_ms: null, count: 0 },
];

/** The row of a work time: the first bucket at least as long as it. */
function heatRow(ms: number): number {
  const k = Math.ceil(Math.log2(Math.max(ms, 1e-9) / 0.25));
  return Math.min(Math.max(k, 0), LIVE_BUCKETS.length - 1);
}

/** One node's work times over `ticks` as a `bucket -> count` map (from the per-node histograms). */
export function nodeHist(ticks: readonly Tick[], node: number): Map<number, number> {
  const out = new Map<number, number>();
  for (const t of ticks)
    for (const [b, c] of t.nodes[node]?.hist ?? []) out.set(b, (out.get(b) ?? 0) + c);
  return out;
}

/**
 * A node's latency over the window for the heatmap: `windows` columns of `LIVE_BUCKETS` rows, the
 * node's own hops that finished in each stretch of ticks. Empty with no per-node histogram.
 */
export function nodeHeat(ticks: readonly Tick[], node: number, windows = 60): number[][] {
  if (ticks.length === 0) return [];
  const per = Math.max(1, Math.ceil(ticks.length / windows));
  const out: number[][] = [];
  for (let i = 0; i < ticks.length; i += per) {
    const col = Array.from({ length: LIVE_BUCKETS.length }, () => 0);
    for (const t of ticks.slice(i, i + per))
      for (const [b, c] of t.nodes[node]?.hist ?? []) {
        const r = heatRow(bucketNs(b) / 1e6);
        col[r] = (col[r] ?? 0) + c;
      }
    out.push(col);
  }
  return out;
}
