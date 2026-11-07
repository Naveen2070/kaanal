import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../../data/api/types";
import { nodeChangedLine, nodeGroupDiff, nodeSetups } from "./nodeSetups";

const svc = (more: Record<string, unknown> = {}): NodeDoc => ({ type: "service", ...more });
const db = (more: Record<string, unknown> = {}): NodeDoc => ({
  type: "database",
  pool: { size: 20, queue: 50 },
  ...more,
});
const ids = (c: NodeDoc, name = "app") => nodeSetups(name, c).map((s) => s.id);
const get = (c: NodeDoc, id: string, name = "app") => nodeSetups(name, c).find((s) => s.id === id)!;

test("a_service_offers_its_server_styles_fail_fast_and_a_request_limit", () => {
  expect(ids(svc({ concurrency: { limit: 12, queue: 100 } }))).toEqual([
    "event-loop",
    "thread-per-request",
    "fail-fast",
    "request-limit",
  ]);
});

test("a_database_offers_its_two_published_pool_sizes_not_server_styles", () => {
  expect(ids(db(), "db")).toEqual(["small-pool", "postgres-pool", "fail-fast", "request-limit"]);
});

test("a_component_with_no_slots_has_no_fail_fast", () => {
  expect(ids(svc())).toEqual(["event-loop", "thread-per-request", "request-limit"]);
});

test("every_setup_says_where_its_number_comes_from_or_that_it_has_none", () => {
  for (const s of nodeSetups("db", db())) expect(s.note.length, s.id).toBeGreaterThan(20);
  expect(get(db(), "small-pool", "db").note).toContain("HikariCP");
  expect(get(db(), "postgres-pool", "db").note).toContain("max_connections");
  expect(get(svc(), "thread-per-request").note).toContain("Tomcat");
  expect(get(svc(), "request-limit").note).toContain("placeholder");
  expect(get({ type: "storage" }, "request-limit", "files").note).toContain("S3");
});

test("event_loop_is_the_untouched_state_and_thread_per_request_is_pressed_once_chosen", () => {
  expect(get(svc(), "event-loop").pressed).toBe(true);
  expect(get(svc(), "thread-per-request").pressed).toBe(false);
  expect(get(svc({ hold: "call" }), "event-loop").pressed).toBe(false);
  expect(get(svc({ hold: "call" }), "thread-per-request").pressed).toBe(true);
});

test("thread_per_request_with_no_limit_writes_tomcats_defaults_in_one_patch", () => {
  expect(get(svc(), "thread-per-request").ops).toEqual([
    { op: "insert", path: ["nodes", "app", "hold"], value: "call" },
    { op: "insert", path: ["nodes", "app", "concurrency"], value: { limit: 200, queue: 100 } },
  ]);
});

test("event_loop_removes_the_hold_and_does_nothing_when_there_is_none", () => {
  expect(get(svc({ hold: "call" }), "event-loop").ops).toEqual([
    { op: "remove", path: ["nodes", "app", "hold"] },
  ]);
  expect(get(svc(), "event-loop").ops).toEqual([]);
});

test("fail_fast_sets_the_queue_to_zero_and_is_pressed_at_zero", () => {
  const c = svc({ concurrency: { limit: 12, queue: 100 } });
  expect(get(c, "fail-fast").pressed).toBe(false);
  expect(get(c, "fail-fast").ops).toEqual([
    { op: "set", path: ["nodes", "app", "concurrency", "queue"], value: 0 },
  ]);
  expect(get(svc({ concurrency: { limit: 12, queue: 0 } }), "fail-fast").pressed).toBe(true);
});

test("the_pool_sizes_set_the_pool_and_are_pressed_at_their_size", () => {
  expect(get(db(), "small-pool", "db").ops).toEqual([
    { op: "set", path: ["nodes", "db", "pool", "size"], value: 10 },
  ]);
  expect(get(db({ pool: { size: 10, queue: 5 } }), "small-pool", "db").pressed).toBe(true);
  expect(get(db({ pool: { size: 100, queue: 5 } }), "postgres-pool", "db").pressed).toBe(true);
});

test("a_request_limit_starts_where_the_add_button_does_and_is_pressed_once_set", () => {
  expect(get(svc(), "request-limit").ops).toEqual([
    { op: "insert", path: ["nodes", "app", "rate_limit"], value: { rps: 1000, burst: 1000 } },
  ]);
  expect(get({ type: "storage" }, "request-limit", "files").ops).toEqual([
    { op: "insert", path: ["nodes", "files", "rate_limit"], value: { rps: 5500, burst: 5500 } },
  ]);
  const set = svc({ rate_limit: { rps: 50, burst: 10 } });
  expect(get(set, "request-limit").pressed).toBe(true);
  expect(get(set, "request-limit").ops).toEqual([]);
});

test("the_changed_line_names_the_groups_that_hold_a_setting_in_the_panels_order", () => {
  expect(nodeChangedLine(svc())).toBe("Nothing changed from the defaults.");
  expect(
    nodeChangedLine(svc({ hold: "call", rate_limit: { rps: 1, burst: 1 }, balance: "random" })),
  ).toBe("Changed from the defaults: Picking a copy, Server style, Request limit");
  expect(nodeChangedLine(svc({ bandwidth: { out: "1MB/s" } }))).toBe(
    "Changed from the defaults: Network card",
  );
});

test("the_diff_rows_list_only_what_a_group_changes_from_its_default", () => {
  expect(nodeGroupDiff("style", svc())).toEqual([]);
  expect(nodeGroupDiff("style", svc({ hold: "call" }))).toEqual([
    { label: "Server style", was: "Event loop", now: "Thread per request" },
  ]);
  expect(nodeGroupDiff("limit", svc({ rate_limit: { rps: 100, burst: 20 } }))).toEqual([
    { label: "Request limit", was: "none", now: "100 / s" },
    { label: "Burst", was: "none", now: "20" },
  ]);
  expect(nodeGroupDiff("balance", svc({ balance: "p2c" }))).toEqual([
    { label: "Picking a copy", was: "round robin", now: "power of two choices" },
  ]);
  expect(nodeGroupDiff("network", svc({ bandwidth: { out: "2MB/s" } }))).toEqual([
    { label: "Sends up to", was: "no limit", now: "2 MB/s" },
  ]);
});

test("a_gateway_with_a_written_chain_names_steps_in_the_changed_line", () => {
  expect(nodeChangedLine(svc(), ["Steps"])).toBe("Changed from the defaults: Steps");
  expect(nodeChangedLine(svc({ hold: "call" }), ["Steps"])).toBe(
    "Changed from the defaults: Server style, Steps",
  );
  expect(nodeChangedLine(svc(), [])).toBe("Nothing changed from the defaults.");
});
