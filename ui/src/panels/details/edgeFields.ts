// A wire's settings as data: where each lives in the config and the patch that edits it, as
// `fields.ts` does for a node. The loader's limits are in `EDGE_LIMITS`; a value outside them would
// make the draft invalid, so the fields never send one. `net` written as a spread (p50 and p99, a
// mean) is shown but not edited here.

import type { Doc, EdgeDoc, Op } from "../../data/api/types";
import { durationMs, formatMs } from "./fields";
import { parseBytes } from "./units";

export const EDGE_LIMITS = {
  netMs: { min: 0, max: 60_000 },
  /** The loader caps a repeated call at 1000. */
  calls: { min: 1, max: 1000 },
  /** Percent of requests that take the wire; above 0. */
  sharePct: { min: 0.01, max: 100 },
  bandwidthKb: { min: 0.001, max: 1_000_000_000 },
} as const;

const put = (i: number, field: string, e: EdgeDoc, value: unknown): Op => ({
  op: field in e ? "set" : "insert",
  path: ["edges", i, field],
  value,
});

/** The wire's network latency when it is a plain duration; null when absent or a spread. */
export function netMsOf(e: EdgeDoc): { ms: number | null; spread: boolean } {
  if (!("net" in e)) return { ms: null, spread: false };
  const ms = durationMs(e.net);
  return { ms, spread: ms === null };
}

export const setNet = (i: number, e: EdgeDoc, ms: number): Op => put(i, "net", e, formatMs(ms));

export const callsOf = (e: EdgeDoc): number => (typeof e.calls === "number" ? e.calls : 1);
export const setCalls = (i: number, e: EdgeDoc, n: number): Op => put(i, "calls", e, n);

/** The share of requests that take the wire, in percent (the config's 0 to 1, times 100). */
export const shareOf = (e: EdgeDoc): number =>
  typeof e.prob === "number" ? Number((e.prob * 100).toFixed(4)) : 100;
export const setShare = (i: number, e: EdgeDoc, pct: number): Op =>
  put(i, "prob", e, Number((pct / 100).toFixed(6)));

/**
 * Whether the config accepts `prob` here: not on a wire out of a gateway, and not on one into a
 * cache (a cache keeps its hit ratio). An unknown end is treated as allowed.
 */
export function shareApplies(doc: Doc | null | undefined, e: EdgeDoc): boolean {
  const kind = (n: unknown) => doc?.nodes?.[String(n)]?.type;
  return kind(e.from) !== "gateway" && kind(e.to) !== "cache";
}

/** The link's capacity in KB/s (1024 bytes), or null when there is no limit. */
export const bandwidthKbOf = (e: EdgeDoc): number | null => {
  const bytes = parseBytes(e.bandwidth, true);
  return bytes === null ? null : Number((bytes / 1024).toFixed(3));
};
/** Bytes per second, at least 1 and whole, as the loader reads them. */
export const setBandwidth = (i: number, e: EdgeDoc, kb: number): Op =>
  put(i, "bandwidth", e, Math.max(1, Math.round(kb * 1024)));
export const clearBandwidth = (i: number, e: EdgeDoc): Op[] =>
  "bandwidth" in e ? [{ op: "remove", path: ["edges", i, "bandwidth"] }] : [];
