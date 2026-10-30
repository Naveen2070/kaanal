// Pure helpers between the draft's JSON and what the canvas draws: ids, unique names, the
// connect patch, and the first layout. Node names have no character rule (`#`, `/`, spaces,
// unicode), so every id that leaves this file is encoded and decoded before an API call.

import type { Doc, Layout, Op } from "../api/types";

export const encodeId = (name: string): string => encodeURIComponent(name);
export const decodeId = (id: string): string => decodeURIComponent(id);

/** `base`, then `base-2`, `base-3`, ... the first that is not in `taken`. */
export function uniqueName(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) if (!used.has(`${base}-${n}`)) return `${base}-${n}`;
}

/** The patch that draws an edge: one more item of `edges`, or the key itself when there is none. */
export function connectOps(doc: Doc, from: string, to: string): Op[] {
  const edge = { from, to };
  const ops: Op[] = Array.isArray(doc.edges)
    ? [{ op: "append", path: ["edges"], value: edge }]
    : [{ op: "edges" in doc ? "set" : "insert", path: ["edges"], value: [edge] }];

  const fromNode = doc.nodes?.[from];
  const toNode = doc.nodes?.[to];
  const isGateway = fromNode?.type === "gateway";
  const canReceiveTraffic = !toNode || toNode.type === "service" || toNode.type === "monolith";

  if (isGateway && canReceiveTraffic && doc.endpoints && typeof doc.endpoints === "object") {
    for (const [epKey, epVal] of Object.entries(doc.endpoints)) {
      if (epVal && typeof epVal === "object") {
        const epObj = epVal as Record<string, unknown>;
        const hasChain = Array.isArray(epObj.chain) && epObj.chain.length > 0;
        if (!hasChain) {
          const opKind = "chain" in epObj ? "set" : "insert";
          ops.push({
            op: opKind,
            path: ["endpoints", epKey, "chain"],
            value: [{ call: to }],
          });
          break;
        }
      }
    }
  }

  return ops;
}

/** Index of the last edge from `from` to `to` (a bare YAML end may be a number: compare as text). */
export function lastEdgeIndex(doc: Doc, from: string, to: string): number {
  const edges = doc.edges ?? [];
  for (let i = edges.length - 1; i >= 0; i--) {
    const e = edges[i];
    if (e && String(e.from) === from && String(e.to) === to) return i;
  }
  return -1;
}

const COL_W = 280;
const ROW_H = 150;

/**
 * Left to right by depth from the nodes nothing points at, rows in the order of `nodes`. Returns
 * a position for every node (the caller keeps only the ones that have none).
 */
export function autoLayout(doc: Doc): Layout {
  const names = Object.keys(doc.nodes ?? {});
  const out = new Map<string, string[]>(names.map((n) => [n, []]));
  const hasIn = new Set<string>();
  for (const e of doc.edges ?? []) {
    const [from, to] = [String(e.from), String(e.to)];
    out.get(from)?.push(to);
    hasIn.add(to);
  }
  const depth = new Map<string, number>();
  const queue = names.filter((n) => !hasIn.has(n));
  // a graph that is all cycles still gets drawn: start from its first node
  if (queue.length === 0 && names[0] !== undefined) queue.push(names[0]);
  for (const n of queue) depth.set(n, 0);
  for (let i = 0; i < queue.length; i++) {
    const n = queue[i] as string;
    for (const m of out.get(n) ?? []) {
      if (!depth.has(m)) {
        depth.set(m, (depth.get(n) ?? 0) + 1);
        queue.push(m);
      }
    }
  }
  const rows = new Map<number, number>();
  const layout: Layout = {};
  for (const n of names) {
    const d = depth.get(n) ?? Math.max(0, ...depth.values()) + 1;
    const row = rows.get(d) ?? 0;
    rows.set(d, row + 1);
    layout[n] = [d * COL_W, row * ROW_H];
  }
  return layout;
}
