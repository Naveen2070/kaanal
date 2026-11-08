import { expect, test } from "vite-plus/test";
import type { LinkSummary, Summary } from "../data/api/types";
import { nodeSummary } from "../test/summary";
import {
  compareRuns,
  copiesNeeded,
  fixFirstLimit,
  fixNode,
  planFor,
  reportMarkdown,
  sloVerdict,
  whyNodeRed,
  whyWireRed,
} from "./insights";

const link = (over: Partial<LinkSummary> = {}): LinkSummary => ({
  from: "gw",
  to: "app",
  transfers: 10,
  mean_wait_ms: 5,
  offered_load: 3,
  utilization: 1,
  saturated: true,
  saturates_at_req_s: 100,
  series: { window_s: 1, latency: [] },
  ...over,
});

const summary = (over: Partial<Summary> = {}): Summary => ({
  label: "run",
  window_s: 10,
  requests: 1000,
  failed: 10,
  errors: {},
  warnings: [],
  latency: { p50_ms: 5, p95_ms: 20, p99_ms: 40, max_ms: 90, buckets: [] },
  series: {} as Summary["series"],
  streams: [],
  nodes: [],
  links: [],
  bottlenecks: [],
  ...over,
});

const hot = nodeSummary({
  name: "shop",
  saturated: true,
  offered_load: 2,
  visits_per_s: 300,
  capacity_per_s: 150,
  max_queue: 40,
  limited: 5,
  overflowed: 0,
  replicas: [
    { replica: 0, visits_per_s: 150, utilization: 1, mean_wait_ms: 3 },
    { replica: 1, visits_per_s: 150, utilization: 1, mean_wait_ms: 3 },
  ] as never,
});

test("a_saturated_component_says_what_arrives_what_it_finishes_and_who_waits", () => {
  const why = whyNodeRed(hot);
  expect(why).toContain("shop gets 300/s");
  expect(why).toContain("finishes at most 150/s");
  expect(why).toContain("up to 40 wait in line");
  expect(why).toContain("5 were turned away");
  expect(whyNodeRed(nodeSummary({ saturated: false }))).toBeNull();
});

test("copies_are_sized_to_stay_under_seventy_percent_busy", () => {
  // 2 copies at 200% busy: 2 * 2 / 0.7 = 5.71, so 6
  expect(copiesNeeded(hot, 1)).toBe(6);
  expect(fixNode(hot)).toContain("2 now, about 6");
  expect(fixNode(nodeSummary({ saturated: false }))).toBeNull();
});

test("a_full_wire_explains_why_what_is_behind_it_looks_idle", () => {
  const why = whyWireRed(link(), 280);
  expect(why).toContain("gw → app");
  expect(why).toContain("carries 280 KB/s");
  expect(why).toContain("300%");
  expect(why).toContain("can look idle");
  expect(whyWireRed(link({ saturated: false }), 280)).toBeNull();
});

test("the_first_limit_names_the_step_for_a_wire_or_a_component", () => {
  const s = summary({
    nodes: [{ ...hot, saturates_at_req_s: 500 }],
    links: [link({ saturates_at_req_s: 100 })],
  });
  const fix = fixFirstLimit(s, () => 280);
  expect(fix?.name).toBe("gw → app");
  expect(fix?.text).toContain("at least");
  // with no bandwidth known there is no step to name
  expect(fixFirstLimit(s, () => null)).toBeNull();
  expect(fixFirstLimit(undefined, () => null)).toBeNull();
});

test("compare_lists_only_the_figures_that_moved", () => {
  const a = summary();
  const b = summary({ latency: { ...a.latency, p99_ms: 4100 }, failed: 0 });
  const lines = compareRuns(a, b);
  expect(lines.join(" | ")).toContain("p99 40 ms → 4.1 s");
  expect(lines.join(" | ")).toContain("errors 10 → 0");
  expect(compareRuns(a, summary())).toEqual([]);
});

test("the_target_verdict_says_within_or_over_by_how_much", () => {
  expect(sloVerdict(40, 100)).toContain("within the 100 ms target (60% to spare)");
  expect(sloVerdict(150, 100)).toContain("over the 100 ms target by 50 ms");
  expect(sloVerdict(40, null)).toBeNull();
});

test("the_plan_lists_only_what_would_need_more_at_a_higher_rate", () => {
  const s = summary({
    nodes: [
      { ...hot, saturates_at_req_s: 150, offered_load: 1.5 },
      nodeSummary({ name: "db", saturates_at_req_s: 100000, offered_load: 0.01 }),
    ],
    links: [link({ saturates_at_req_s: 100 })],
  });
  const rows = planFor(s, 300, () => 280);
  const names = rows.map((r) => r.name);
  expect(names).toContain("shop");
  expect(names).toContain("gw → app");
  expect(names).not.toContain("db");
  // 2 copies, load 300/150 = 2x capacity: 2 * 2 / 0.7 = 5.7, so 6
  expect(rows.find((r) => r.name === "shop")?.text).toBe("2 → about 6 copies");
  expect(planFor(s, 10, () => 280)).toEqual([]);
});

test("the_report_is_markdown_with_the_what_if_label_and_the_ranking", () => {
  const md = reportMarkdown("Run 3", summary({ warnings: ["w1"], errors: { timeout: 4 } }), [
    { name: "gw → app", peak: 1, saturatesAtReqS: 100, wire: true },
    { name: "db", peak: 0.03, saturatesAtReqS: null },
  ]);
  expect(md).toContain("# Run 3");
  expect(md).toContain("not a prediction");
  expect(md).toContain("| p99 | 40 ms |");
  expect(md).toContain("| gw → app (wire) | 100% | 100 req/s |");
  expect(md).toContain("| db | 3% | no saturation point |");
  expect(md).toContain("- w1");
  expect(md).toContain("- timeout: 4");
});

test("a_cpu_bound_component_names_its_cpus_and_asks_for_more_cores", () => {
  const n = nodeSummary({
    name: "shop",
    saturated: true,
    offered_load: null,
    utilization: null,
    visits_per_s: 150,
    cpu_load: 1.5,
    cpu_capacity_per_s: 100,
    saturates_by: "cpu",
  });
  expect(whyNodeRed(n)).toContain("finishes at most 100/s");
  expect(whyNodeRed(n)).toContain("Its CPUs are the limit, not its slots.");
  const fix = fixNode(n);
  expect(fix).toContain("More CPU for shop");
  expect(fix).toContain("1.5 times the cores");
});
