import { expect, test } from "vite-plus/test";
import { nodeSummary } from "../test/summary";
import {
  chaosBody,
  deleteOps,
  durationMs,
  queueOf,
  setQueue,
  setSlots,
  slotsOf,
  suggestedFix,
  worksOutTo,
} from "./fields";

test("works_out_to_matches_slots_over_service_time", () => {
  // 12 at once / 98 ms each = 122.4 per second
  expect(worksOutTo(12, 98, 1).capacityPerS).toBeCloseTo(122.4, 1);
  expect(worksOutTo(12, 98, 3).capacityPerS).toBeCloseTo(367.3, 1);
});

test("spare_capacity_is_negative_when_offered_load_exceeds_capacity", () => {
  expect(worksOutTo(10, 100, 1, 50).spare).toBeCloseTo(0.5);
  expect(worksOutTo(10, 100, 1, 100).spare).toBeCloseTo(0);
  expect(worksOutTo(10, 100, 1, 150).spare).toBeLessThan(0);
});

test("suggested_fix_names_the_cause", () => {
  expect(suggestedFix(undefined)).toBeNull();
  expect(suggestedFix(nodeSummary())).toBeNull();
  expect(suggestedFix(nodeSummary({ saturated: true }))).toMatch(/Add instances/);
  expect(suggestedFix(nodeSummary({ saturated: true, overflowed: 4 }))).toMatch(/queue limit/);
  expect(suggestedFix(nodeSummary({ timed_out: 2 }))).toMatch(/deadline/);
});

test("each_control_patches_the_right_path", () => {
  const num = { type: "service", concurrency: 8, proc: "5ms" };
  expect(setSlots("app", num, 16)).toEqual([
    { op: "set", path: ["nodes", "app", "concurrency"], value: 16 },
  ]);
  const long = { type: "service", concurrency: { limit: 8, queue: 4 } };
  expect(setSlots("app", long, 16)).toEqual([
    { op: "set", path: ["nodes", "app", "concurrency", "limit"], value: 16 },
  ]);
  const pool = { type: "database", pool: { size: 20 } };
  expect(setSlots("db", pool, 30)).toEqual([
    { op: "set", path: ["nodes", "db", "pool", "size"], value: 30 },
  ]);
  expect(setSlots("x", { type: "service" }, 4)).toEqual([
    { op: "insert", path: ["nodes", "x", "concurrency"], value: 4 },
  ]);
  // a queue limit
  expect(setQueue("app", long, 9)).toEqual([
    { op: "set", path: ["nodes", "app", "concurrency", "queue"], value: 9 },
  ]);
  expect(setQueue("app", num, 9)).toEqual([
    { op: "set", path: ["nodes", "app", "concurrency"], value: { limit: 8, queue: 9 } },
  ]);
  expect(setQueue("db", pool, 5)).toEqual([
    { op: "insert", path: ["nodes", "db", "pool", "queue"], value: 5 },
  ]);
  expect(setQueue("x", { type: "service" }, 5)).toEqual([]);
});

test("slots_and_queue_are_read_from_either_form", () => {
  expect(slotsOf({ type: "s", concurrency: 8 })).toEqual({ value: 8, from: "concurrency" });
  expect(slotsOf({ type: "s", concurrency: { limit: 3 } }).value).toBe(3);
  expect(slotsOf({ type: "d", pool: { size: 20 } })).toEqual({ value: 20, from: "pool" });
  expect(slotsOf({ type: "s" }).value).toBeNull();
  expect(queueOf({ type: "s", concurrency: { limit: 3, queue: 7 } })).toBe(7);
  expect(queueOf({ type: "s", concurrency: 3 })).toBe(0);
  expect(queueOf({ type: "s" })).toBeNull();
});

test("only_a_plain_duration_is_editable", () => {
  expect(durationMs("98ms")).toBe(98);
  expect(durationMs("2s")).toBe(2000);
  expect(durationMs("1.5s")).toBe(1500);
  expect(durationMs({ p50: "4ms", p99: "20ms" })).toBeNull();
  expect(durationMs(undefined)).toBeNull();
});

test("deleting_a_node_removes_its_edges_highest_index_first", () => {
  const edges = [
    { from: "gw", to: "app" },
    { from: "app", to: "db" },
    { from: "gw", to: "db" },
    { from: "app", to: "cache" },
  ];
  expect(deleteOps("app", edges)).toEqual([
    { op: "remove", path: ["nodes", "app"] },
    { op: "remove", path: ["edges", 3] },
    { op: "remove", path: ["edges", 1] },
    { op: "remove", path: ["edges", 0] },
  ]);
});

test("chaos_targets_are_structured_objects_never_strings", () => {
  expect(chaosBody({ node: "db", replica: 1 }, { kind: "down", down: true })).toEqual({
    node: "db",
    replica: 1,
    down: true,
  });
  expect(chaosBody({ edge: { from: "app", to: "db" } }, { kind: "slow", ms: 20 })).toEqual({
    edge: { from: "app", to: "db" },
    latency_ms: 20,
  });
  expect(chaosBody({ node: "app" }, { kind: "drop", rate: 0.1 })).toEqual({
    node: "app",
    error_rate: 0.1,
  });
  expect(chaosBody({ node: "app" }, { kind: "clear" })).toEqual({
    node: "app",
    down: false,
    latency_ms: 0,
    error_rate: 0,
  });
  for (const v of Object.values(chaosBody({ edge: { from: "a", to: "b" } }, { kind: "clear" }))) {
    expect(typeof v === "string" && v.includes("->")).toBe(false);
  }
});
