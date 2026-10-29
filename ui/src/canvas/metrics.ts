// What a card shows, worked out from one node's run figures and its config. Pure, so the numbers
// are tested without drawing anything. Honest metrics (spec section 6): the busy bar is measured
// utilization, the peak is the highest of the ~60 windows, the sparkline is queue depth (exact),
// and headroom is capacity, not a prediction.

import type { NodeDoc, NodeSummary } from "../api/types";

export interface Figures {
  /** Measured utilization 0..1; null without a run or without slots. */
  busy: number | null;
  /** The highest windowed busy fraction; null without a busy series. */
  peak: number | null;
  /** Longest queue per window. */
  queue: number[];
  /** The longest queue seen in the run. */
  waiting: number | null;
  /** "saturates near 120 req/s"; null without a capacity. */
  headroomText: string | null;
  saturated: boolean;
  /** Copies of the node (1 when the config says nothing). */
  replicas: number;
  /** True once a run has given any figure for this node. */
  hasRun: boolean;
}

/** Two significant digits: 122.4 gives 120, 1530 gives 1500, 7.46 gives 7.5. */
export function twoSignificant(n: number): string {
  return Number(n.toPrecision(2)).toLocaleString("en-US");
}

export function replicasOf(config: NodeDoc | undefined): number {
  const r = config?.replicas;
  return typeof r === "number" && Number.isInteger(r) && r >= 1 ? r : 1;
}

export function nodeFigures(node: NodeSummary | undefined, config: NodeDoc | undefined): Figures {
  const replicas = replicasOf(config);
  if (!node) {
    return {
      busy: null,
      peak: null,
      queue: [],
      waiting: null,
      headroomText: null,
      saturated: false,
      replicas,
      hasRun: false,
    };
  }
  const busy = node.series.busy;
  const req = node.saturates_at_req_s;
  return {
    busy: node.utilization,
    peak: busy.length > 0 ? Math.max(...busy) : null,
    queue: node.series.queue,
    waiting: node.max_queue,
    headroomText: req !== null && req > 0 ? `saturates near ${twoSignificant(req)} req/s` : null,
    saturated: node.saturated,
    replicas,
    hasRun: true,
  };
}

/** A whole percent, or an en dash when there is nothing to show. */
export function percent(x: number | null): string {
  return x === null ? "–" : `${Math.round(x * 100)}%`;
}
