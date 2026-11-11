import { expect, test } from "vite-plus/test";
import type { Doc } from "../data/api/types";
import type { Tick } from "../data/api/liveTypes";
import { EMPTY_FEED } from "./feed";
import { liveFeed } from "./liveFeed";

const doc = {
  nodes: {
    gw: { type: "gateway" },
    app: { type: "service", concurrency: 4, replicas: 2 },
    db: { type: "database", cpu: { cores: 2 } },
  },
  edges: [],
} as unknown as Doc;

const dropped = { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0 };
const tick = (
  t_ms: number,
  app: { busy: number; queue?: number; arrived?: number; drop?: number },
): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: 10,
  failed: 0,
  nodes: [
    { arrived: 10, dropped, queue: 0, busy_slots: 0, cpu_load: null },
    {
      arrived: app.arrived ?? 10,
      dropped: { ...dropped, overflow: app.drop ?? 0 },
      queue: app.queue ?? 0,
      busy_slots: app.busy,
      cpu_load: null,
    },
    { arrived: 10, dropped, queue: 0, busy_slots: 0, cpu_load: 0.5 },
  ],
  wires: [],
  hist: [],
  journeys: [],
  status: { rate: 1, paused: false, overloaded: false, in_flight: 0 },
});

test("without_ticks_or_a_file_the_feed_is_empty", () => {
  expect(liveFeed([], doc)).toBe(EMPTY_FEED);
  expect(liveFeed([tick(250, { busy: 1 })], null)).toBe(EMPTY_FEED);
});

test("per_second_figures_are_over_the_last_five_seconds", () => {
  // 20 ticks (5 s), 10 arrivals each: 40 a second
  const ticks = Array.from({ length: 30 }, (_, i) => tick((i + 1) * 250, { busy: 2 }));
  const feed = liveFeed(ticks, doc);
  expect(feed.node("app")?.visits_per_s).toBeCloseTo(40, 5);
  expect(feed.peakVisits).toBeCloseTo(40, 5);
  expect(feed.nodes.map((n) => n.name)).toEqual(["gw", "app", "db"]);
});

test("fullness_is_slots_in_use_over_slots_times_copies_else_cores_in_use", () => {
  const feed = liveFeed([tick(250, { busy: 4 })], doc);
  // 4 busy of 4 slots x 2 copies
  expect(feed.node("app")?.utilization).toBe(0.5);
  // the database has cores and no slots: its cpu load
  expect(feed.node("db")?.utilization).toBe(0.5);
  // a gateway with neither has no figure
  expect(feed.node("gw")?.utilization).toBeNull();
});

test("a_full_node_a_queue_or_a_drop_makes_it_saturated", () => {
  expect(liveFeed([tick(250, { busy: 1 })], doc).node("app")?.saturated).toBe(false);
  expect(liveFeed([tick(250, { busy: 8 })], doc).node("app")?.saturated).toBe(true);
  expect(liveFeed([tick(250, { busy: 1, queue: 3 })], doc).node("app")?.saturated).toBe(true);
  const f = liveFeed([tick(250, { busy: 1, drop: 2 })], doc).node("app");
  expect(f?.saturated).toBe(true);
  expect(f?.overflowed).toBe(2);
});

test("the_sparkline_is_the_last_minute", () => {
  const ticks = Array.from({ length: 300 }, (_, i) => tick((i + 1) * 250, { busy: 4 }));
  const s = liveFeed(ticks, doc).node("app")?.series;
  expect(s?.busy).toHaveLength(240);
  expect(s?.busy.every((b) => b === 0.5)).toBe(true);
});

test("ticks_are_mapped_to_components_by_name_not_by_the_order_of_the_drafts_json", () => {
  // the draft JSON is alphabetical (app, db, nginx); a tick lists components in file order
  const alphabetical = {
    nodes: {
      app: { type: "service", concurrency: 4 },
      db: { type: "database", concurrency: 4 },
      nginx: { type: "gateway" },
    },
    edges: [],
  } as unknown as Doc;
  const t = (queue: number[], busy: number[]): Tick => ({
    ...tick(250, { busy: 0 }),
    nodes: [0, 1, 2].map((i) => ({
      arrived: 10,
      dropped,
      queue: queue[i] ?? 0,
      busy_slots: busy[i] ?? 0,
      cpu_load: null,
    })),
  });
  // file order nginx, app, db: the app has the queue and its slot full
  const ticks = [t([0, 592, 0], [0, 4, 1])];
  const feed = liveFeed(ticks, alphabetical, ["nginx", "app", "db"]);
  expect(feed.node("app")?.series.queue.at(-1)).toBe(592);
  expect(feed.node("app")?.utilization).toBe(1);
  expect(feed.node("db")?.series.queue.at(-1)).toBe(0);
  expect(feed.node("db")?.utilization).toBe(0.25);
  expect(feed.node("nginx")?.series.queue.at(-1)).toBe(0);
  // the strip's busiest is the app, with its own queue
  const busiest = [...feed.nodes].sort((a, b) => (b.utilization ?? 0) - (a.utilization ?? 0))[0];
  expect(busiest?.name).toBe("app");
});

test("a_hello_that_lists_other_components_than_the_draft_is_ignored_for_the_missing_ones", () => {
  const feed = liveFeed([tick(250, { busy: 1 })], doc, ["gw", "app", "db", "ghost"]);
  expect(feed.nodes.map((n) => n.name)).toEqual(["gw", "app", "db"]);
});

test("the_real_source_reads_arrivals_and_drops_from_tick_real_and_keeps_the_engine_snapshot", () => {
  const t = tick(250, { busy: 6, queue: 4, arrived: 10, drop: 2 });
  const real = {
    requests: 3,
    failed: 1,
    nodes: [
      { arrived: 3, dropped, queue: 0, busy_slots: 0, cpu_load: null },
      { arrived: 3, dropped: { ...dropped, overflow: 1 }, queue: 0, busy_slots: 0, cpu_load: null },
      { arrived: 0, dropped, queue: 0, busy_slots: 0, cpu_load: null },
    ],
    wires: [],
    hist: [],
  };
  const withReal = [{ ...t, real }];
  const all = liveFeed(withReal, doc).node("app");
  const only = liveFeed(withReal, doc, undefined, "real").node("app");
  // 5 s window at 4 ticks a second is one tick here; per second figures scale the same way
  expect((only?.visits_per_s ?? 0) / (all?.visits_per_s ?? 1)).toBeCloseTo(3 / 10);
  expect(only?.overflowed).toBe(1);
  expect(all?.overflowed).toBe(2);
  // the queue and the slots in use are the engine's, whichever requests made them
  expect(only?.max_queue).toBe(all?.max_queue);
  // a tick with no real request reads as nothing arriving
  const none = liveFeed([t], doc, undefined, "real").node("app");
  expect(none?.visits_per_s).toBe(0);
});

test("the_limit_is_the_rate_over_the_busy_share_by_the_utilization_law", () => {
  // app: 4 slots x 2 copies, 2 in use = 25% busy at 40 requests a second: busy 1 at 160 a second
  const ticks = [250, 500, 750, 1000].map((t) => tick(t, { busy: 2 }));
  const f = liveFeed(ticks, doc);
  expect(f.node("app")?.saturates_at_req_s).toBeCloseTo(160);
  // a node with nothing running gives no figure, rather than an infinite one
  expect(f.node("gw")?.saturates_at_req_s).toBeNull();
});

test("a_node_reads_its_work_times_and_its_copies_from_the_ticks", () => {
  const bucket = (ms: number) => Math.floor(Math.log((ms * 1e6) / 1e4) / Math.log(1.0201)) + 1;
  const withData = (t: Tick): Tick => {
    const app = t.nodes[1] as Tick["nodes"][number];
    return {
      ...t,
      nodes: [
        t.nodes[0] as Tick["nodes"][number],
        { ...app, hist: [[bucket(8), 10]], replicas: [7, 3] },
        t.nodes[2] as Tick["nodes"][number],
      ],
    };
  };
  const ticks = [250, 500, 750, 1000].map((t) => withData(tick(t, { busy: 2 })));
  const app = liveFeed(ticks, doc).node("app");
  // 40 hops at 8 ms: the median and the 99th both read 8 ms, to a bucket's 1%
  expect(app?.p50_ms).toBeCloseTo(8, 0);
  expect(app?.p99_ms).toBeCloseTo(8, 0);
  // two copies, 28 and 12 hops over 1 s: 28/s and 12/s
  expect(app?.replicas.map((r) => Math.round(r.visits_per_s))).toEqual([28, 12]);
  expect(app?.series.latency.length).toBeGreaterThan(0);
  // a node with no histogram has no latency to read
  expect(liveFeed(ticks, doc).node("gw")?.p99_ms).toBeNull();
});
