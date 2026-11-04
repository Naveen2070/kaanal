// What the draft holds, for the stats pill: parts by sort, and wires. Pure, so it is tested alone.
// Traffic is what takes requests in (gateways), compute is what works on them (services, monoliths,
// auth), data is what stores (databases, caches), and external APIs are counted on their own.

import type { Doc } from "../../data/api/types";

export interface Counts {
  components: number;
  traffic: number;
  compute: number;
  data: number;
  external: number;
  connections: number;
}

export function systemCounts(doc: Doc | null | undefined): Counts {
  const kinds = Object.values(doc?.nodes ?? {}).map((n) => n.type);
  const n = (...of: string[]) => kinds.filter((k) => of.includes(String(k))).length;
  return {
    components: kinds.length,
    traffic: n("gateway", "load-balancer"),
    compute: n("service", "monolith", "auth"),
    data: n("database", "cache", "storage"),
    external: n("external-api"),
    connections: Array.isArray(doc?.edges) ? doc.edges.length : 0,
  };
}

/**
 * What the status line says about the run: "re-running..." while one is in flight, "re-run in 0.4 s"
 * (how long the newest one took) once there is one, nothing before the first. `blocked` says
 * nothing here: why it could not run is a notice of its own.
 */
export function runLine(
  status: "idle" | "running" | "done" | "blocked",
  run: { started_ms: number; finished_ms: number | null } | null,
): string | null {
  if (status === "running") return "re-running...";
  if (run === null || typeof run.finished_ms !== "number" || typeof run.started_ms !== "number") {
    return null;
  }
  const ms = Math.max(0, run.finished_ms - run.started_ms);
  return `re-run in ${Number((ms / 1000).toPrecision(2))} s`;
}
