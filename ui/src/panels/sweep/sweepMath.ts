// The sweep chart's maths, pure so it is tested without drawing. Load factor k is a geometric ladder
// (1, 2, 4, ...) and p99 spans decades, so both axes are logarithmic: v maps to
// from + (ln v - ln min) / (ln max - ln min) * (to - from).

import type { SweepPointDoc, SweepResultDoc, SweepVerdict } from "../../data/api/types";

export type FirstToSaturate = SweepResultDoc["first_to_saturate"];

const FLOOR = 1e-3; // a log axis needs a positive value; 1 microsecond in ms

/** A log scale from [min, max] onto [from, to]. A zero-width domain maps to the middle. */
export function logScale(
  min: number,
  max: number,
  from: number,
  to: number,
): (v: number) => number {
  const lo = Math.log(Math.max(min, FLOOR));
  const span = Math.log(Math.max(max, FLOOR)) - lo;
  if (!(span > 0)) return () => (from + to) / 2;
  const [a, b] = from < to ? [from, to] : [to, from];
  // clamped: a value below the domain (a zero p99) or Infinity stays on the plot edge
  return (v) =>
    Math.min(b, Math.max(a, from + ((Math.log(Math.max(v, FLOOR)) - lo) / span) * (to - from)));
}

/** The p99 axis: every p99 and the limit, with a little air so no mark sits on the frame. */
export function p99Domain(points: SweepPointDoc[], limitMs: number): [number, number] {
  const v = [limitMs, ...points.map((p) => p.p99_ms)].filter((x) => x > 0 && Number.isFinite(x));
  if (v.length === 0) return [1, 10]; // nothing positive to scale: a safe finite decade
  const lo = Math.min(...v);
  const hi = Math.max(...v);
  return [lo / 1.25, hi * 1.25];
}

export interface SweepDot {
  x: number;
  y: number;
  over: boolean;
  k: number;
}

/** Points in ascending load order (a copy); the server sorts, but the chart must not rely on it. */
export const byK = (points: SweepPointDoc[]) => [...points].sort((a, b) => a.k - b.k);

export function sweepGeometry(
  input: SweepPointDoc[],
  limitMs: number,
  size: { w: number; h: number },
): { line: string; dots: SweepDot[]; limitY: number; kneeX: number | null } {
  const points = byK(input);
  const ks = points.map((p) => p.k);
  const x = logScale(Math.min(...ks), Math.max(...ks), 0, size.w);
  const [lo, hi] = p99Domain(points, limitMs);
  const y = logScale(lo, hi, size.h, 0);
  const dots = points.map((p) => ({
    x: x(p.k),
    y: y(p.p99_ms),
    over: p.over.p99 || p.over.errors,
    k: p.k,
  }));
  // the knee is the last passing point before the first crossing (as the server defines it)
  const fi = dots.findIndex((d) => d.over);
  return {
    line: dots.map((d) => `${d.x.toFixed(1)},${d.y.toFixed(1)}`).join(" "),
    dots,
    limitY: y(limitMs > 0 ? limitMs : lo), // NaN or <= 0: no real limit, the component draws no line
    kneeX: fi > 0 ? (dots[fi - 1] as SweepDot).x : null,
  };
}

const at = (k: number, reqS: number) => `${reqS.toFixed(0)} req/s (${k.toFixed(2)}x)`;

/** The finding as one sentence. */
export function finding(v: SweepVerdict | null, limitMs: number, first: FirstToSaturate): string {
  if (!v) return "Load sweep: no result yet.";
  const where = at(v.k, v.req_s);
  const head =
    v.kind === "knee"
      ? `knee at ${where}, the last load within the limits`
      : v.kind === "broken_at_lowest"
        ? `over a limit already at the lowest point, ${where}`
        : `no failure seen up to ${where}`;
  const sat = first ? `; first to saturate: ${first.node} at ${at(first.k, first.req_s)}` : "";
  const lim = limitMs > 0 && Number.isFinite(limitMs) ? `p99 limit ${limitMs} ms, plus the ` : "";
  return `Load sweep: ${head} (${lim}error-rate limit)${sat}.`;
}

export const describeSweep = (r: SweepResultDoc) =>
  finding(r.verdict, r.limit.p99_ms, r.first_to_saturate).replace(
    /\.$/,
    r.stopped_at != null ? `; the ladder was cut at ${r.stopped_at}x.` : ".",
  );
