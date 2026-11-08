import { describe, expect, it, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import { nodeSummary } from "../../test/summary";
import {
  balanceOf,
  coresOf,
  hasCpu,
  setCores,
  carriesEstimates,
  cardOf,
  chaosBody,
  startMsOf,
  controlRows,
  deleteOps,
  durationMs,
  firstByteEditable,
  firstByteMs,
  clearQueueLimit,
  queueOf,
  setBalance,
  setCard,
  setFlavor,
  setThroughput,
  throughputKb,
  hasProc,
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
  expect(queueOf({ type: "s", concurrency: 3 })).toBe("unbounded"); // a bare slot count never turns anyone away for a full queue
  expect(queueOf({ type: "s", concurrency: { limit: 3, queue: 0 } })).toBe(0); // 0 is nobody waits
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

test("the_starting_at_box_is_seconds_in_and_milliseconds_out", () => {
  expect(startMsOf("")).toBeUndefined(); // empty: from the start
  expect(startMsOf("  ")).toBeUndefined();
  expect(startMsOf("4")).toBe(4000);
  expect(startMsOf("0.25")).toBe(250);
  expect(startMsOf("0")).toBe(0);
});

test("a_starting_at_value_the_loader_would_refuse_is_not_a_value", () => {
  for (const bad of ["-1", "abc", "1e400", "86401", "NaN"]) expect(startMsOf(bad), bad).toBeNull();
});

test("a_start_is_sent_only_with_taking_a_node_or_instance_down", () => {
  expect(chaosBody({ node: "db" }, { kind: "down", down: true, fromMs: 4000 })).toEqual({
    node: "db",
    down: true,
    from_ms: 4000,
  });
  expect(chaosBody({ node: "db", replica: 1 }, { kind: "down", down: true, fromMs: 250 })).toEqual({
    node: "db",
    replica: 1,
    down: true,
    from_ms: 250,
  });
  // a link outage has no start, bringing a node back has none, and no start is no key
  expect(
    chaosBody({ edge: { from: "a", to: "b" } }, { kind: "down", down: true, fromMs: 1 }),
  ).toEqual({ edge: { from: "a", to: "b" }, down: true });
  expect(chaosBody({ node: "db" }, { kind: "down", down: false, fromMs: 1 })).toEqual({
    node: "db",
    down: false,
  });
  expect(chaosBody({ node: "db" }, { kind: "down", down: true })).toEqual({
    node: "db",
    down: true,
  });
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

// ---- T3.1: the rows the control bar shows, per node kind (R1.99) ----

const ids = (kind: string, config: Record<string, unknown> = {}) =>
  controlRows(kind, { type: kind, ...config }).map((r) => r.id);

test("gateway_service_monolith_auth_get_time_slots_copies", () => {
  for (const kind of ["gateway", "service", "monolith", "auth"]) {
    expect(ids(kind, { proc: "5ms" }), kind).toEqual(["time", "slots", "copies"]);
  }
});

test("external_api_gets_time_slots_copies", () => {
  expect(ids("external-api", { proc: "120ms", pool: { size: 8 } })).toEqual([
    "time",
    "slots",
    "copies",
  ]);
});

test("database_and_cache_have_no_time_row", () => {
  // their cost comes from the flavor (ops, hit, miss): a `proc` patch would make the draft invalid
  expect(ids("database", { pool: { size: 10 } })).toEqual(["slots", "copies"]);
  expect(ids("cache")).toEqual(["slots", "copies"]);
});

test("a_distribution_proc_is_read_only_with_the_yaml_note", () => {
  const rows = controlRows("service", { type: "service", proc: { p50: "4ms", p99: "20ms" } });
  const time = rows.find((r) => r.id === "time")!;
  expect(time.editable).toBe(false);
  expect(time.why).toBe("edit in kaanal.yaml");
  // the others stay editable
  expect(rows.filter((r) => r.id !== "time").every((r) => r.editable && r.why === undefined)).toBe(
    true,
  );
});

test("a_plain_duration_or_no_proc_at_all_is_editable", () => {
  for (const cfg of [{ proc: "25ms" }, { proc: "2s" }, {}]) {
    const time = controlRows("service", { type: "service", ...cfg }).find((r) => r.id === "time")!;
    expect(time.editable, JSON.stringify(cfg)).toBe(true);
  }
  // text that is not a duration is not something the bar can rewrite safely
  const odd = controlRows("service", { type: "service", proc: "fast" }).find(
    (r) => r.id === "time",
  )!;
  expect(odd.editable).toBe(false);
});

test("slots_maps_to_concurrency_limit_or_pool_size", () => {
  expect(slotsOf({ type: "service", concurrency: 12 }).value).toBe(12);
  expect(slotsOf({ type: "service", concurrency: { limit: 7, queue: 3 } }).value).toBe(7);
  expect(slotsOf({ type: "database", pool: { size: 10 } })).toEqual({ value: 10, from: "pool" });
  expect(slotsOf({ type: "service" }).value).toBeNull();
});

test("an_unknown_kind_gets_no_time_row_rather_than_a_guess", () => {
  expect(ids("mystery")).toEqual(["slots", "copies"]);
});

test("clear_queue_limit_turns_the_long_form_back_into_a_bare_slot_count", () => {
  const ops = clearQueueLimit("s", { type: "service", concurrency: { limit: 3, queue: 7 } });
  expect(ops).toHaveLength(1);
  expect(ops[0]).toMatchObject({ op: "set", value: 3 });
  // a bare count is already unbounded; a pool always has a queue
  expect(clearQueueLimit("s", { type: "service", concurrency: 3 })).toEqual([]);
  expect(clearQueueLimit("d", { type: "database", pool: { size: 5, queue: 2 } })).toEqual([]);
});

describe("a node's network card", () => {
  const svc = (extra: Record<string, unknown> = {}) => ({ type: "service", ...extra }) as NodeDoc;
  it("reads one number as both sides and a pair as each", () => {
    expect(cardOf(svc({ bandwidth: 2048 }))).toEqual({ send: 2, receive: 2 });
    expect(cardOf(svc({ bandwidth: { out: 1024 } }))).toEqual({ send: 1, receive: null });
    expect(cardOf(svc())).toEqual({ send: null, receive: null });
    expect(cardOf(svc({ bandwidth: "250KB/s" }))).toEqual({ send: 250, receive: 250 });
    expect(cardOf(svc({ bandwidth: { out: "1MB/s", in: 262144 } }))).toEqual({
      send: 1024,
      receive: 256,
    });
    expect(cardOf(svc({ bandwidth: "soon" }))).toEqual({ send: null, receive: null });
  });
  it("writes one number for equal sides, a pair otherwise, and removes when none is left", () => {
    expect(setCard("a", svc(), "send", 1)).toEqual([
      { op: "insert", path: ["nodes", "a", "bandwidth"], value: { out: 1024 } },
    ]);
    expect(setCard("a", svc({ bandwidth: { out: 1024 } }), "receive", 1)).toEqual([
      { op: "set", path: ["nodes", "a", "bandwidth"], value: 1024 },
    ]);
    expect(setCard("a", svc({ bandwidth: 1024 }), "send", 2)).toEqual([
      { op: "set", path: ["nodes", "a", "bandwidth"], value: { out: 2048, in: 1024 } },
    ]);
    expect(setCard("a", svc({ bandwidth: { out: 1024 } }), "send", null)).toEqual([
      { op: "remove", path: ["nodes", "a", "bandwidth"] },
    ]);
    expect(setCard("a", svc(), "send", null)).toEqual([]);
  });
});

test("a_load_balancer_has_a_service_time_and_its_spread_is_a_balance", () => {
  expect(hasProc("load-balancer")).toBe(true);
  expect(balanceOf("random")).toBe("random");
  expect(setBalance(["nodes", "lb", "spread"], false, "random")).toEqual([
    { op: "insert", path: ["nodes", "lb", "spread"], value: "random" },
  ]);
  expect(setBalance(["nodes", "lb", "spread"], true, "")).toEqual([
    { op: "remove", path: ["nodes", "lb", "spread"] },
  ]);
});

test("least_conn_and_p2c_are_balances_the_page_can_write", () => {
  expect(balanceOf("least-conn")).toBe("least-conn");
  expect(balanceOf("p2c")).toBe("p2c");
  expect(setBalance(["nodes", "a", "balance"], true, "p2c")).toEqual([
    { op: "set", path: ["nodes", "a", "balance"], value: "p2c" },
  ]);
});

test("storage_fields_read_the_files_text_and_write_whole_bytes", () => {
  expect(hasProc("storage")).toBe(false);
  expect(throughputKb({ type: "storage", throughput: "80MB/s" })).toBe(81_920);
  expect(throughputKb({ type: "storage", throughput: "250KB/s" })).toBe(250);
  expect(throughputKb({ type: "storage", throughput: 2048 })).toBe(2);
  expect(throughputKb({ type: "storage" })).toBeNull();
  expect(throughputKb({ type: "storage", throughput: "fast" })).toBeNull();
  expect(setThroughput("s", { type: "storage" }, 100)).toEqual([
    { op: "insert", path: ["nodes", "s", "throughput"], value: 102_400 },
  ]);
  expect(setThroughput("s", { type: "storage", throughput: "80MB/s" }, 0.0001)).toEqual([
    { op: "set", path: ["nodes", "s", "throughput"], value: 1 },
  ]);
  expect(setThroughput("s", { type: "storage", throughput: "80MB/s" }, null)).toEqual([
    { op: "remove", path: ["nodes", "s", "throughput"] },
  ]);
  expect(setThroughput("s", { type: "storage" }, null)).toEqual([]);
  expect(setFlavor("s", { type: "storage" }, "blob")).toEqual({
    op: "insert",
    path: ["nodes", "s", "flavor"],
    value: "blob",
  });
  expect(firstByteMs({ type: "storage", first_byte: "20ms" })).toBe(20);
  expect(firstByteEditable({ type: "storage", first_byte: { p50: "25ms", p99: "100ms" } })).toBe(
    false,
  );
});

test("only_the_starter_values_count_as_estimates", () => {
  const starter = {
    type: "storage",
    first_byte: "25ms",
    throughput: "80MB/s",
  };
  expect(carriesEstimates(starter)).toBe(true);
  expect(carriesEstimates({ ...starter, throughput: 83_886_080 })).toBe(true);
  expect(carriesEstimates({ ...starter, throughput: "81MB/s" })).toBe(false);
  expect(carriesEstimates({ ...starter, first_byte: "26ms" })).toBe(false);
  expect(carriesEstimates({ ...starter, first_byte: { p50: "25ms", p99: "100ms" } })).toBe(false);
  expect(carriesEstimates({ type: "storage" })).toBe(false);
});

describe("cores", () => {
  const svc: NodeDoc = { type: "service" };

  test("only_the_kinds_whose_compute_is_sized_have_a_cpu_box", () => {
    for (const t of ["gateway", "service", "monolith", "database", "auth", "load-balancer"])
      expect(hasCpu(t), t).toBe(true);
    for (const t of ["cache", "external-api", "storage"]) expect(hasCpu(t), t).toBe(false);
  });

  test("coresOf_reads_a_positive_number_and_nothing_else", () => {
    expect(coresOf({ ...svc, cpu: { cores: 0.5 } })).toBe(0.5);
    expect(coresOf(svc)).toBeNull();
    expect(coresOf({ ...svc, cpu: { cores: 0 } })).toBeNull();
    expect(coresOf({ ...svc, cpu: { cores: "x" } })).toBeNull();
    expect(coresOf({ ...svc, cpu: 4 })).toBeNull();
  });

  test("setCores_inserts_replaces_and_removes_in_one_op", () => {
    expect(setCores("app", svc, 4)).toEqual([
      { op: "insert", path: ["nodes", "app", "cpu"], value: { cores: 4 } },
    ]);
    expect(setCores("app", { ...svc, cpu: { cores: 4 } }, 2)).toEqual([
      { op: "set", path: ["nodes", "app", "cpu"], value: { cores: 2 } },
    ]);
    expect(setCores("app", { ...svc, cpu: { cores: 4 } }, null)).toEqual([
      { op: "remove", path: ["nodes", "app", "cpu"] },
    ]);
    expect(setCores("app", svc, null)).toEqual([]);
  });
});
