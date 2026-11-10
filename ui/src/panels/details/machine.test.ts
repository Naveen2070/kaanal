import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import type { Doc } from "../../data/api/types";
import {
  CUSTOM_NAME,
  DEFAULT_RESTART_MS,
  addCustomInstance,
  customInstancesOf,
  gcOf,
  instanceOf,
  memoryOf,
  onLimitOf,
  removeCustomInstance,
  restartMsOf,
  setCustomInstance,
  setGc,
  setInstance,
  setMemory,
  setOnLimit,
  setRestart,
  usersOfInstance,
} from "./fields";
import { cpuSummary } from "./groups/summaries";

const node = (o: Record<string, unknown> = {}): NodeDoc => ({ type: "service", ...o }) as NodeDoc;

test("the_instance_is_set_replaced_and_removed", () => {
  expect(setInstance("app", node(), "aws/m5.large")).toEqual([
    { op: "insert", path: ["nodes", "app", "instance"], value: "aws/m5.large" },
  ]);
  expect(setInstance("app", node({ instance: "aws/m5.large" }), "gcp/e2-medium")).toEqual([
    { op: "set", path: ["nodes", "app", "instance"], value: "gcp/e2-medium" },
  ]);
  expect(setInstance("app", node({ instance: "aws/m5.large" }), null)).toEqual([
    { op: "remove", path: ["nodes", "app", "instance"] },
  ]);
  expect(setInstance("app", node(), null)).toEqual([]);
  expect(instanceOf(node({ instance: "aws/m5.large" }))).toBe("aws/m5.large");
  expect(instanceOf(node({ instance: "" }))).toBeNull();
});

test("memory_boxes_write_the_map_whole_and_remove_it_when_empty", () => {
  expect(setMemory("app", node(), "per_request", 1048576)).toEqual([
    { op: "insert", path: ["nodes", "app", "memory"], value: { per_request: 1048576 } },
  ]);
  const had = node({ memory: { limit: "8GB", per_request: 1024 } });
  // a size the file wrote as text is kept as it is when another box changes
  expect(setMemory("app", had, "base", 2048)).toEqual([
    {
      op: "set",
      path: ["nodes", "app", "memory"],
      value: { limit: "8GB", per_request: 1024, base: 2048 },
    },
  ]);
  expect(setMemory("app", node({ memory: { per_request: 5 } }), "per_request", null)).toEqual([
    { op: "remove", path: ["nodes", "app", "memory"] },
  ]);
  expect(setMemory("app", node(), "limit", null)).toEqual([]);
});

test("memoryOf_reads_numbers_and_leaves_text_to_the_file", () => {
  expect(memoryOf(node({ memory: { limit: 1000, per_request: "1MB" } }))).toEqual({
    limit: 1000,
    per_request: null,
    base: null,
  });
  expect(memoryOf(node())).toEqual({ limit: null, per_request: null, base: null });
});

test("the_summary_names_cores_instance_and_memory", () => {
  expect(cpuSummary(node())).toBe("not limited");
  expect(
    cpuSummary(node({ cpu: { cores: 2 }, instance: "aws/m5.large", memory: { limit: 1 } })),
  ).toBe("2 cores · aws/m5.large · memory limit");
});

test("on_limit_kill_is_written_with_a_restart_and_refuse_takes_both_out", () => {
  const withMem = node({ memory: { limit: 1, per_request: 1 } });
  expect(onLimitOf(withMem)).toBe("refuse");
  expect(setOnLimit("app", withMem, "kill")).toEqual([
    {
      op: "set",
      path: ["nodes", "app", "memory"],
      value: { limit: 1, per_request: 1, on_limit: "kill" },
    },
  ]);
  const killing = node({ memory: { limit: 1, per_request: 1, on_limit: "kill", restart: "30s" } });
  expect(onLimitOf(killing)).toBe("kill");
  expect(restartMsOf(killing)).toBe(30000);
  expect(restartMsOf(withMem)).toBe(DEFAULT_RESTART_MS);
  expect(setRestart("app", killing, 5000)).toEqual([
    {
      op: "set",
      path: ["nodes", "app", "memory"],
      value: { limit: 1, per_request: 1, on_limit: "kill", restart: "5000ms" },
    },
  ]);
  expect(setOnLimit("app", killing, "refuse")).toEqual([
    { op: "set", path: ["nodes", "app", "memory"], value: { limit: 1, per_request: 1 } },
  ]);
  expect(setOnLimit("app", node(), "kill")).toEqual([]);
});

test("the_gc_weight_is_written_into_the_memory_map_and_taken_out_again", () => {
  const withMem = node({ memory: { limit: 1, per_request: 1 } });
  expect(gcOf(withMem)).toBeNull();
  expect(setGc("app", withMem, 0.2)).toEqual([
    {
      op: "set",
      path: ["nodes", "app", "memory"],
      value: { limit: 1, per_request: 1, gc: 0.2 },
    },
  ]);
  const had = node({ memory: { limit: 1, per_request: 1, gc: 0.2 } });
  expect(gcOf(had)).toBe(0.2);
  expect(setGc("app", had, null)).toEqual([
    { op: "set", path: ["nodes", "app", "memory"], value: { limit: 1, per_request: 1 } },
  ]);
  expect(setGc("app", node(), 0.2)).toEqual([]);
});

test("custom_instances_are_added_edited_and_deleted_as_patches_of_the_instances_map", () => {
  const empty = { nodes: { app: { type: "service" } } } as unknown as Doc;
  expect(addCustomInstance(empty, "box", 8)).toEqual([
    { op: "insert", path: ["instances"], value: { box: { cores: 8 } } },
  ]);
  const doc = {
    instances: { box: { cores: 8, memory: "64GB" }, rack: { cores: 2 } },
    nodes: { app: { type: "service", instance: "box" } },
  } as unknown as Doc;
  expect(addCustomInstance(doc, "tiny", 0.5)).toEqual([
    { op: "insert", path: ["instances", "tiny"], value: { cores: 0.5 } },
  ]);
  expect(Object.keys(customInstancesOf(doc))).toEqual(["box", "rack"]);
  // a size the file wrote as text reads as none here and is kept when another box changes
  expect(customInstancesOf(doc).box).toEqual({ cores: 8, memory: null, bandwidth: null });
  expect(setCustomInstance(doc, "box", "bandwidth", 1048576)).toEqual([
    {
      op: "set",
      path: ["instances", "box"],
      value: { cores: 8, memory: "64GB", bandwidth: 1048576 },
    },
  ]);
  expect(setCustomInstance(doc, "box", "memory", 0)).toEqual([
    { op: "set", path: ["instances", "box"], value: { cores: 8 } },
  ]);
  expect(setCustomInstance(doc, "box", "cores", null)).toEqual([]);
  expect(setCustomInstance(doc, "nope", "cores", 4)).toEqual([]);
  // an instance a node names cannot be deleted; an unused one can
  expect(usersOfInstance(doc, "box")).toEqual(["app"]);
  expect(removeCustomInstance(doc, "box")).toEqual([]);
  expect(removeCustomInstance(doc, "rack")).toEqual([
    { op: "remove", path: ["instances", "rack"] },
  ]);
  expect(CUSTOM_NAME.test("build-box.1")).toBe(true);
  expect(CUSTOM_NAME.test("aws/m5")).toBe(false);
  expect(CUSTOM_NAME.test("")).toBe(false);
});
