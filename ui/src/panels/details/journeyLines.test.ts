import { expect, test } from "vite-plus/test";
import type { Journey, Tick } from "../../data/api/liveTypes";
import { journeyLines } from "./journeyLines";

const hop = (
  node: number,
  start: number,
  end: number,
  wait = 0,
  kind = "proc",
  edge: number | null = null,
) => ({
  node,
  replica: 0,
  kind,
  start_ns: start,
  end_ns: end,
  wait_ns: wait,
  attempt: 1,
  edge,
});
const tick = (t_ms: number, journeys: Journey[]) => ({ t_ms, journeys }) as unknown as Tick;
const edges = [{ from: "gw" }];

test("a_journey_becomes_one_line_a_hop_with_wire_hops_naming_the_caller", () => {
  const j: Journey = {
    hops: [hop(1, 0, 2e6, 1e6), hop(0, 2e6, 3e6, 0, "net", 0)],
    dropped_at: null,
    total_ns: 3e6,
  };
  const lines = journeyLines([tick(1000, [j])], ["gw", "app"], edges);
  expect(lines.map((l) => [l.node, l.event, l.from])).toEqual([
    ["app", "queued", undefined],
    ["gw", "served", "gw"],
  ]);
  expect(lines[0]?.wait_ms).toBe(1);
  expect(lines[0]?.service_ms).toBe(1);
  expect(lines[0]?.t_ms).toBeCloseTo(997); // arrival = tick end - total
});

test("a_dropped_journey_ends_in_a_failure_line_and_nothing_after_it", () => {
  const j: Journey = {
    hops: [hop(0, 0, 1e6), hop(1, 1e6, 2e6), hop(0, 2e6, 3e6)],
    dropped_at: 1,
    total_ns: 3e6,
  };
  const lines = journeyLines([tick(500, [j])], ["gw", "app"], edges);
  expect(lines.map((l) => l.event)).toEqual(["served", "failed"]);
  expect(lines[1]?.level).toBe("error");
});

test("newest_first_and_capped_by_requests", () => {
  const j: Journey = { hops: [hop(0, 0, 1e6)], dropped_at: null, total_ns: 1e6 };
  const lines = journeyLines([tick(250, [j, j]), tick(500, [j, j])], ["gw"], edges, 3);
  expect(new Set(lines.map((l) => l.request)).size).toBe(3);
  expect(lines[0]?.request).toBe(500 * 1000 + 1); // the last journey of the last tick
});
