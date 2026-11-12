import { expect, test } from "vite-plus/test";
import type { Tick } from "../api/liveTypes";
import {
  bucketNs,
  bucketOf,
  breakerBands,
  endpointFigures,
  errorKinds,
  liveSeries,
  nodeHeat,
  nodeHist,
  nodeVisitRates,
  quantileMs,
  recent,
  sumHist,
  timelineSeries,
  windowFigures,
  wireFigures,
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

test("a_tick_that_arrives_changes_only_the_last_point_and_a_new_session_never_reads_an_old_one", () => {
  const mk = (n: number, requests: number) =>
    Array.from({ length: n }, (_, i) => tick((i + 1) * 250, { requests, failed: 0, hist: [] }));
  const a = mk(996, 25); // 10 ticks a point, and one more tick keeps it so
  const first = timelineSeries(a, 100);
  const again = timelineSeries([...a, tick(249250, { requests: 25, failed: 0, hist: [] })], 100);
  // finished chunks are the very same points; only the tail differs
  expect(again.slice(0, 50)).toEqual(first.slice(0, 50));
  // a restarted session has the same t_ms but other counts: nothing stale comes back
  const b = timelineSeries(mk(996, 50), 100);
  expect(b[3]?.reqPerS).toBeCloseTo(200, 5);
  expect(first[3]?.reqPerS).toBeCloseTo(100, 5);
});

test("errorKinds_adds_every_tick_and_orders_by_count", () => {
  const n = (d: Partial<Tick["nodes"][number]["dropped"]>) => ({
    arrived: 0,
    dropped: { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0, ...d },
    queue: 0,
    busy_slots: 0,
    cpu_load: null,
  });
  const t = (d: Parameters<typeof n>[0]) => ({ nodes: [n(d)] }) as unknown as Tick;
  expect(errorKinds([t({ overflow: 2 }), t({ overflow: 1, deadline: 5 })])).toEqual([
    ["timed out", 5],
    ["queue full", 3],
  ]);
});

test("liveSeries_gives_rates_a_second_and_nothing_for_a_short_window", () => {
  const t = (i: number) =>
    ({
      t_ms: i * 250,
      requests: 25,
      failed: 5,
      hist: [],
      nodes: [],
      journeys: [],
    }) as unknown as Tick;
  expect(liveSeries([t(1)])).toBeUndefined();
  const s = liveSeries([t(1), t(2), t(3), t(4)]);
  expect(s?.done.at(-1)).toBeCloseTo(80); // (25 - 5) a tick, four ticks a second
  expect(s?.failed.at(-1)).toBeCloseTo(20);
});

test("wireFigures_read_crossings_mean_time_and_mean_wait_per_crossing", () => {
  const t = (i: number, carried: number, wire_ns: number, wait_ns: number) =>
    ({ t_ms: i * 250, wires: [{ carried, wire_ns, wait_ns }] }) as unknown as Tick;
  const f = wireFigures([t(1, 10, 10e6, 5e6), t(2, 10, 30e6, 15e6)], 0);
  expect(f.perS).toBeCloseTo(40); // 20 crossings in 0.5 s
  expect(f.meanWireMs).toBeCloseTo(2); // 40 ms over 20 crossings
  expect(f.meanWaitMs).toBeCloseTo(1);
  expect(wireFigures([t(1, 0, 0, 0)], 0).meanWireMs).toBeNull();
});

test("nodeHist_and_nodeHeat_read_one_nodes_work_times", () => {
  const b = (ms: number) => bucketOf(ms * 1e6);
  const t = (i: number, hist: [number, number][]) =>
    ({ t_ms: i * 250, nodes: [{ hist: [] }, { hist }] }) as unknown as Tick;
  const ticks = [
    t(1, [[b(1.5), 10]]),
    t(2, [
      [b(1.5), 10],
      [b(40), 1],
    ]),
  ];
  const sum = nodeHist(ticks, 1);
  expect([...sum.values()].reduce((a, c) => a + c, 0)).toBe(21);
  // 1.5 ms falls under "<= 2 ms" (row 3: 0.25, 0.5, 1, 2) and 40 ms under "<= 64 ms" (row 8)
  const heat = nodeHeat(ticks, 1, 2);
  expect(heat).toHaveLength(2);
  expect(heat[0]?.[3]).toBe(10);
  expect(heat[1]?.[8]).toBe(1);
  expect(
    nodeHeat(ticks, 0, 2)
      .flat()
      .reduce((a, c) => a + c, 0),
  ).toBe(0);
});

test("wireFigures_count_a_retry_on_the_wire_its_first_try_came_in_through", () => {
  const t = (carried: number, retries: number, retried: number) =>
    ({
      t_ms: 250,
      wires: [{ carried, wire_ns: 0, wait_ns: 0, retries, retried }],
    }) as unknown as Tick;
  // 12 crossings of which 2 were retries: 10 calls, 2 of them retried once each
  const f = wireFigures([t(12, 2, 2)], 0);
  expect(f.retriedPct).toBeCloseTo(20);
  expect(f.retriesPerCall).toBeCloseTo(0.2);
  expect(wireFigures([t(5, 0, 0)], 0).retriedPct).toBeNull();
});

test("wireFigures_add_the_engines_retry_funnel_and_average_the_wire_load", () => {
  const t = (ms: number, retried: number, recovered: number, load?: number) =>
    tick(ms, {
      wires: [
        {
          carried: 10,
          funnel: { calls: 10, retried, recovered, failed: retried - recovered },
          ...(load === undefined ? {} : { load }),
        },
      ],
    });
  const f = wireFigures([t(250, 4, 3, 0.2), t(500, 6, 2, 0.6)], 0);
  // 10 retried, 5 recovered: 5 gave up (or are still retrying)
  expect(f.funnel).toEqual({ calls: 20, retried: 10, recovered: 5, gaveUp: 5 });
  expect(f.load).toBeCloseTo(0.4, 12);
  // no retry scope and no bandwidth: nothing to show
  const plain = wireFigures([tick(250, { wires: [{ carried: 3 }] })], 0);
  expect([plain.funnel, plain.load]).toEqual([null, null]);
  // recovered can run ahead of retried across a tick edge: never a negative count
  expect(wireFigures([t(250, 1, 3)], 0).funnel?.gaveUp).toBe(0);
});

test("endpointFigures_read_one_route_over_the_window", () => {
  const b = bucketOf(20e6);
  const t = (ms: number) =>
    tick(ms, {
      endpoints: [
        { requests: 5, failed: 1, hist: [[b, 4]] },
        { requests: 2, failed: 0, hist: [[bucketOf(300e6), 2]] },
      ],
    });
  const w = [t(250), t(500), t(750), t(1000)];
  const first = endpointFigures(w, 0);
  expect(first.reqPerS).toBe(20); // 20 requests in a second
  expect(first.failedPerS).toBe(4);
  expect(Math.abs((first.p99Ms ?? 0) - 20)).toBeLessThan(0.3);
  expect(Math.abs((endpointFigures(w, 1).p50Ms ?? 0) - 300)).toBeLessThan(4);
  // a route that nothing finished on
  expect(endpointFigures(w, 5)).toEqual({ reqPerS: 0, failedPerS: 0, p50Ms: null, p99Ms: null });
});

test("breakerBands_follow_each_breaker_of_a_wire_tick_by_tick", () => {
  const at = (ms: number, state: "closed" | "open" | "half_open") =>
    tick(ms, {
      breakers: [
        { edge: 2, replica: 0, state },
        { edge: 3, replica: 0, state: "open" },
      ],
    });
  const bands = breakerBands(
    [at(250, "closed"), at(500, "open"), at(750, "open"), at(1000, "half_open")],
    2,
  );
  expect(bands).toHaveLength(1);
  expect(bands[0]?.phases).toEqual(["closed", "open", "open", "half_open"]);
  expect(bands[0]?.openS).toBe(0.75);
  expect(bands[0]?.now).toBe("half_open");
  // a wire with no breaker has no band
  expect(breakerBands([at(250, "open")], 9)).toEqual([]);
});

test("wireFigures_read_p50_and_p99_of_the_crossing_time_from_the_wire_histograms", () => {
  const b1 = bucketOf(1e6);
  const b2 = bucketOf(50e6);
  const t = (ms: number) =>
    tick(ms, {
      wires: [
        {
          carried: 10,
          hist: [
            [b1, 9],
            [b2, 1],
          ],
        },
      ],
    });
  const f = wireFigures([t(250), t(500)], 0);
  // 18 requests near 1 ms and 2 near 50 ms: p50 is the 1 ms bucket, p99 the 50 ms one
  expect(Math.abs((f.p50Ms ?? 0) - 1)).toBeLessThan(0.02);
  expect(Math.abs((f.p99Ms ?? 0) - 50)).toBeLessThan(1);
  // an older server sends no histogram: no figure
  const old = wireFigures([tick(250, { wires: [{ carried: 3 }] })], 0);
  expect([old.p50Ms, old.p99Ms]).toEqual([null, null]);
});

test("visits_a_second_per_node_are_the_arrivals_over_the_span_the_ticks_cover", () => {
  const node = (arrived: number) => ({ arrived }) as Tick["nodes"][number];
  // four ticks of 250 ms (one second): 10 arrivals at node a, 4 at b in each of them
  const ticks = [250, 500, 750, 1000].map((t) => tick(t, { nodes: [node(10), node(4)] }));
  expect(nodeVisitRates(ticks, ["a", "b"])).toEqual({ a: 40, b: 16 });
  // only the last 500 ms count when the window is that short
  expect(nodeVisitRates(ticks, ["a", "b"], 500)).toEqual({ a: 40, b: 16 });
  expect(nodeVisitRates([], ["a"])).toEqual({});
  // a name with no node in the tick is zero, not a crash
  expect(nodeVisitRates(ticks, ["a", "b", "c"]).c).toBe(0);
});
