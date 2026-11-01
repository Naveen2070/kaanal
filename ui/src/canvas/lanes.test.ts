import { expect, test } from "vite-plus/test";
import { nodeSummary } from "../test/summary";
import { laneColour, laneCount, laneLoads, laneOffsets, MAX_LANES } from "./lanes";

const rep = (replica: number, visits_per_s: number) => ({
  replica,
  visits: 0,
  visits_per_s,
  mean_wait_ms: 0,
  offered_load: null,
  utilization: null,
});

test("one_lane_per_copy_up_to_a_cap", () => {
  expect([laneCount(1), laneCount(3), laneCount(64), laneCount(0), laneCount(NaN)]).toEqual([
    1,
    3,
    MAX_LANES,
    1,
    1,
  ]);
});

test("lanes_sit_symmetrically_around_the_wire", () => {
  expect(laneOffsets(1)).toEqual([0]);
  expect(laneOffsets(2)).toEqual([-3, 3]);
  expect(laneOffsets(3)).toEqual([-6, 0, 6]);
});

test("each_copy_has_its_own_colour_and_extra_copies_share_the_last", () => {
  expect(laneColour(0)).toBe("var(--lane-0)");
  expect(laneColour(1)).toBe("var(--lane-1)");
  expect(laneColour(20)).toBe(`var(--lane-${MAX_LANES - 1})`);
});

test("a_lane_follows_its_own_copys_load", () => {
  const t = nodeSummary({ replicas: [rep(1, 25), rep(0, 100)], visits_per_s: 125 });
  expect(laneLoads(2, t, 200)).toEqual([0.5, 0.125]); // sorted by copy, not by arrival
});

test("without_a_figure_for_every_copy_the_nodes_load_is_shared", () => {
  expect(laneLoads(2, nodeSummary({ replicas: [], visits_per_s: 100 }), 200)).toEqual([0.25, 0.25]);
});

test("no_run_or_no_traffic_means_idle_lanes_never_nan", () => {
  expect(laneLoads(2, undefined, 100)).toEqual([0, 0]);
  expect(laneLoads(2, nodeSummary({ visits_per_s: 5 }), 0)).toEqual([0, 0]);
  expect(laneLoads(1, nodeSummary({ visits_per_s: NaN, replicas: [] }), 10)).toEqual([0]);
});
