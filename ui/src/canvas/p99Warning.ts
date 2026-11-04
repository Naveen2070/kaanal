// The "no client waits this long" warning (F3, RULES 56): a run whose system p99 is longer than the
// time a real client or proxy gives up. The limit is the entry node's `timeout` when the draft sets
// one, else 30 s (the low end of the 30 to 60 s where clients and proxies give up; an estimate).
// Frontend only: the run's numbers are untouched.

import type { Doc } from "../data/api/types";
import { durationMs } from "../panels/details/fields";

/** The limit when the entry node sets no `timeout`. An estimate. */
export const DEFAULT_CLIENT_LIMIT_MS = 30_000;

export interface EntryNode {
  name: string;
  /** Its `timeout` in milliseconds; null when unset or not a plain duration. */
  timeoutMs: number | null;
}

/** The node clients reach first: the gateway, else the node with no incoming edge (one with a `spec` first). */
export function entryNode(doc: Doc | null | undefined): EntryNode | null {
  const nodes = Object.entries(doc?.nodes ?? {});
  const gateway = nodes.find(([, n]) => n.type === "gateway");
  const hasIncoming = new Set((doc?.edges ?? []).map((e) => String(e.to)));
  const roots = nodes.filter(([name]) => !hasIncoming.has(name));
  const pick = gateway ?? roots.find(([, n]) => "spec" in n) ?? roots[0];
  return pick ? { name: pick[0], timeoutMs: durationMs(pick[1].timeout) } : null;
}

const secs = (ms: number) => `${Number((ms / 1000).toFixed(1))} s`;

/** The warning for a run's system p99, or null when it is within the limit (or there is no p99). */
export function p99Warning(
  p99Ms: number | null | undefined,
  entryTimeoutMs: number | null,
  entryName?: string,
): string | null {
  if (p99Ms == null || !Number.isFinite(p99Ms)) return null;
  const limit = entryTimeoutMs ?? DEFAULT_CLIENT_LIMIT_MS;
  if (p99Ms <= limit) return null;
  return entryTimeoutMs === null
    ? "No client waits this long: add a queue limit or a timeout"
    : `The p99 (${secs(p99Ms)}) is longer than the timeout of ${entryName ?? "the entry node"} (${secs(entryTimeoutMs)}): add a queue limit or a shorter wait`;
}

/** The warning for a run against a draft, or null. */
export function p99WarningFor(p99Ms: number | null | undefined, doc: Doc | null | undefined) {
  const entry = entryNode(doc);
  return p99Warning(p99Ms, entry?.timeoutMs ?? null, entry?.name);
}
