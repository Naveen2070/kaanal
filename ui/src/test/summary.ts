// Run figures for tests.

import type { NodeSummary } from "../data/api/types";

export function nodeSummary(over: Partial<NodeSummary> = {}): NodeSummary {
  return {
    name: "app",
    replicas: [],
    visits_per_s: 100,
    expected_visits_per_s: 100,
    mean_wait_ms: 0,
    offered_load: 0.5,
    utilization: 0.5,
    saturated: false,
    max_queue: 0,
    capacity_per_s: 200,
    saturates_at_req_s: 200,
    headroom: 0.5,
    p50_ms: 10,
    p99_ms: 20,
    series: { window_s: 1, busy: [0.2, 0.9, 0.4], queue: [0, 3, 1], latency: [] },
    limited: 0,
    overflowed: 0,
    timed_out: 0,
    ...over,
  };
}
