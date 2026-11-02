// The latency heatmap's numbers (spec section 6): time across (about 60 windows), latency bucket
// up, the count of the node's own hops in each cell. Pure, so it is tested without drawing.

import type { Bucket, Series } from "../../data/api/types";

export interface HeatmapData {
  /** Time windows (columns). */
  cols: number;
  /** Latency buckets (rows); row 0 is the fastest. */
  rows: number;
  /** `cells[row][col]`: hops that started in that window and fell in that bucket. */
  cells: number[][];
  /** The largest cell; 0 when nothing was recorded. */
  max: number;
}

export function heatmapData(series: Series): HeatmapData {
  const cols = series.latency.length;
  const rows = series.latency.reduce((n, w) => Math.max(n, w.length), 0);
  const cells = Array.from({ length: rows }, (_, r) =>
    Array.from({ length: cols }, (_, c) => series.latency[c]?.[r] ?? 0),
  );
  let max = 0;
  for (const row of cells) for (const v of row) max = Math.max(max, v);
  return { cols, rows, cells, max };
}

/** 0..1 on a log scale, so a few slow hops stay visible next to millions of fast ones. */
export function intensity(count: number, max: number): number {
  return count <= 0 || max <= 0 ? 0 : Math.log1p(count) / Math.log1p(max);
}

/** A bucket's label: its upper bound ("<= 8 ms"), or "slower" for the last one. */
export function bucketLabel(buckets: Bucket[], row: number): string {
  const le = buckets[row]?.le_ms;
  if (le === undefined || le === null) return "slower";
  return `<= ${Number(le.toPrecision(3))} ms`;
}
