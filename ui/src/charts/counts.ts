// What the draft holds, for the stats pill: parts by sort, and wires. Pure, so it is tested alone.
// Traffic is what takes requests in (gateways), compute is what works on them (services, monoliths,
// auth), data is what stores (databases, caches), and external APIs are counted on their own.

import type { Doc } from "../api/types";

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
    traffic: n("gateway"),
    compute: n("service", "monolith", "auth"),
    data: n("database", "cache"),
    external: n("external-api"),
    connections: Array.isArray(doc?.edges) ? doc.edges.length : 0,
  };
}
