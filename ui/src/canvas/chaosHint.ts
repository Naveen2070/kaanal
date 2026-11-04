// A hint for chaos that may never have been felt: a warm cache answers before the database is
// reached, or no route calls the node, so the run shows no failure. Generic on purpose: it cannot
// know which of the two it is, and it makes no claim about latency.

import type { Summary } from "../data/api/types";

/** One node's live chaos as `GET /chaos` reports it under `effective.nodes`. */
export interface NodeChaos {
  down?: boolean;
  latency_ms?: number;
  faults?: unknown[];
}

const WHY =
  "Requests may never reach it: a cache answers first, or no route calls it. Check Arriving on";

/** One hint per node that has chaos set, when the finished run shows nothing failed. */
export function chaosHiddenHint(
  chaosByNode: Record<string, NodeChaos>,
  run: Pick<Summary, "failed"> | null | undefined,
  /** Each node's kind: a cache is left out, since a cache that is down fails nothing (a miss goes on to the database). */
  kinds: Record<string, string | undefined> = {},
): { node: string; text: string }[] {
  if (!run || run.failed !== 0) return [];
  const out: { node: string; text: string }[] = [];
  for (const [node, c] of Object.entries(chaosByNode)) {
    if (kinds[node] === "cache") continue;
    const ms = c.latency_ms ?? 0;
    if (c.down) {
      out.push({ node, text: `Nothing failed although ${node} is down. ${WHY} ${node}.` });
    } else if ((c.faults?.length ?? 0) > 0) {
      out.push({
        node,
        text: `Nothing failed although ${node} drops some requests. ${WHY} ${node}.`,
      });
    } else if (ms > 0) {
      out.push({
        node,
        text: `${node} has ${ms} ms added, but the run shows no failures; if latency looks unchanged, requests may never reach it: a cache answers first, or no route calls it. Check Arriving on ${node}.`,
      });
    }
  }
  return out;
}
