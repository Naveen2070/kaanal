// The real (frontend) share of the live ticks (step 8A): a tick's own figures include the real
// requests, `tick.real` repeats them for the real ones, and the generated share is the difference.

import { expect, test } from "vite-plus/test";
import type { Tick } from "../api/liveTypes";
import { bucketOf, quantileMs, sumHistOf, windowFigures } from "./liveMath";

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

const b10 = bucketOf(10e6);
const b900 = bucketOf(900e6);

/** 10 requests in the tick: 6 generated at 10 ms, 4 real at 900 ms, one real one dropped. */
const mixed = (t: number) =>
  tick(t, {
    requests: 10,
    failed: 1,
    hist: [
      [b10, 6],
      [b900, 3],
    ],
    real: { requests: 4, failed: 1, nodes: [], wires: [], hist: [[b900, 3]] },
  });

test("real_reads_only_the_real_counts", () => {
  const f = windowFigures([mixed(250), mixed(500), mixed(750), mixed(1000)], 1000, "real");
  // 16 real requests in 1 second, 4 dropped
  expect(f.reqPerS).toBe(16);
  expect(f.droppedPerS).toBe(4);
  expect(f.p99Ms).toBeGreaterThan(800);
});

test("generated_is_all_minus_real_with_the_histogram_subtracted", () => {
  const ticks = [mixed(250), mixed(500), mixed(750), mixed(1000)];
  const f = windowFigures(ticks, 1000, "generated");
  expect(f.reqPerS).toBe(24);
  expect(f.droppedPerS).toBe(0);
  // the slow requests were all real: the generated p99 is the 10 ms bucket
  expect(f.p99Ms).toBeLessThan(11);
  const g = sumHistOf(ticks, "generated");
  expect([...g.entries()]).toEqual([[b10, 24]]);
});

test("all_is_unchanged_by_the_source_argument", () => {
  const ticks = [mixed(250), mixed(500)];
  expect(windowFigures(ticks, 1000, "all")).toEqual(windowFigures(ticks, 1000));
  expect(quantileMs(sumHistOf(ticks, "all"), 0.99)).toBeGreaterThan(800);
});

test("a_tick_without_real_counts_as_zero_real_and_all_generated", () => {
  const ticks = [
    tick(250, { requests: 5, hist: [[b10, 5]] }),
    tick(500, { requests: 5, hist: [[b10, 5]] }),
  ];
  const real = windowFigures(ticks, 1000, "real");
  expect([real.reqPerS, real.droppedPerS, real.p50Ms, real.p99Ms]).toEqual([0, 0, null, null]);
  expect(windowFigures(ticks, 1000, "generated").reqPerS).toBe(
    windowFigures(ticks, 1000, "all").reqPerS,
  );
});
