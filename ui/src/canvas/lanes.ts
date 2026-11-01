// A wire carries one lane per copy of the component it feeds (spec 3, developer 2026-11-01). The
// lane's colour names the copy, its pulse and width follow that copy's own load. Pure, so it is
// tested without drawing.

import type { NodeSummary } from "../api/types";

/** More than this many lanes would be a smear; the rest share the last colour. */
export const MAX_LANES = 8;
/** Pixels between neighbouring lanes. */
export const LANE_GAP = 6;

export const laneCount = (copies: number): number =>
  Math.max(1, Math.min(MAX_LANES, Math.trunc(copies) || 1));

/** Offsets from the wire's middle, in pixels, symmetric: 2 lanes give -3 and 3. */
export function laneOffsets(n: number): number[] {
  return Array.from({ length: n }, (_, k) => (k - (n - 1) / 2) * LANE_GAP);
}

/** The CSS colour of copy `k`. */
export const laneColour = (k: number): string => `var(--lane-${Math.min(k, MAX_LANES - 1)})`;

/**
 * How loaded each lane is, 0 to 1 against the busiest node in the run (`peak`, requests/s). A copy's
 * own figure is used when the run has one for every lane; otherwise the node's load is shared evenly.
 */
export function laneLoads(n: number, target: NodeSummary | undefined, peak: number): number[] {
  if (!target || !(peak > 0)) return Array<number>(n).fill(0);
  const perCopy =
    target.replicas.length === n
      ? [...target.replicas].sort((a, b) => a.replica - b.replica).map((r) => r.visits_per_s)
      : Array<number>(n).fill(target.visits_per_s / n);
  return perCopy.map((v) => (Number.isFinite(v) ? Math.min(Math.max(v / peak, 0), 1) : 0));
}
