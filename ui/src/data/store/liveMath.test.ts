import { expect, test } from "vite-plus/test";
import type { Tick } from "../api/liveTypes";
import {
  bucketNs,
  bucketOf,
  quantileMs,
  recent,
  sumHist,
  timelineSeries,
  windowFigures,
} from "./liveMath";

function tick(t_ms: number, over: Partial<Tick> = {}): Tick {
  return {
    t_ms,
    system_ms: t_ms,
    requests: 0,
    failed: 0,
    nodes: [],
    wires: [],
    hist: [],
    journeys: [],
    status: { rate: 1, paused: false, overloaded: false, in_flight: 0 },
    ...over,
  };
}

test("a_latency_lands_in_a_bucket_whose_middle_is_within_1_percent", () => {
  for (const ms of [0.05, 1, 20, 350, 12_000]) {
    const ns = ms * 1e6;
    const mid = bucketNs(bucketOf(ns));
    expect(Math.abs(mid - ns) / ns, `${ms} ms`).toBeLessThan(0.011);
  }
  expect(bucketOf(5_000)).toBe(0);
});

test("histograms_of_ticks_add_up_and_a_quantile_reads_the_sum", () => {
  const b20 = bucketOf(20e6);
  const b200 = bucketOf(200e6);
  const ticks = [
    tick(250, { hist: [[b20, 90]] }),
    tick(500, {
      hist: [
        [b20, 8],
        [b200, 2],
      ],
    }),
  ];
  const sum = sumHist(ticks);
  expect(sum.get(b20)).toBe(98);
  expect(sum.get(b200)).toBe(2);
  // 100 requests: p50 is in the 20 ms bucket, p99 reaches the 200 ms one
  expect(quantileMs(sum, 0.5)).toBeCloseTo(20, 0);
  expect(quantileMs(sum, 0.99)).toBeCloseTo(200, -1);
  expect(quantileMs(new Map(), 0.5)).toBeNull();
});

test("the_recent_window_is_the_last_milliseconds_of_the_newest_tick", () => {
  const ticks = [250, 500, 750, 1000].map((t) => tick(t));
  expect(recent(ticks, 500).map((t) => t.t_ms)).toEqual([750, 1000]);
  expect(recent([], 500)).toEqual([]);
});

test("window_figures_are_per_second_and_count_drops", () => {
  const b = bucketOf(10e6);
  // four ticks of a second: 100 finished each, 10 dropped each
  const ticks = [250, 500, 750, 1000].map((t) =>
    tick(t, { requests: 25, failed: 2, hist: [[b, 23]] }),
  );
  const f = windowFigures(ticks, 1000);
  expect(f.reqPerS).toBeCloseTo(100, 5);
  expect(f.droppedPerS).toBeCloseTo(8, 5);
  expect(f.p50Ms).toBeCloseTo(10, 0);
  expect(windowFigures([], 1000).p99Ms).toBeNull();
});

test("a_long_window_is_cut_to_at_most_240_points_that_keep_their_sums", () => {
  const ticks = Array.from({ length: 7200 }, (_, i) =>
    tick((i + 1) * 250, { requests: 25, failed: 5, hist: [[bucketOf(20e6), 20]] }),
  );
  const pts = timelineSeries(ticks, 240);
  expect(pts.length).toBeLessThanOrEqual(240);
  expect(pts.length).toBeGreaterThan(100);
  // 25 finished every quarter second is 100 a second, 5 dropped is 20 a second, whatever the cut
  expect(pts[10]?.reqPerS).toBeCloseTo(100, 5);
  expect(pts[10]?.droppedPerS).toBeCloseTo(20, 5);
  expect(pts[10]?.p99Ms).toBeCloseTo(20, 0);
  expect(timelineSeries([])).toEqual([]);
});
