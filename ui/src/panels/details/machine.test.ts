import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import { instanceOf, memoryOf, setInstance, setMemory } from "./fields";
import { INSTANCE_TYPES, instanceByName, instanceLine } from "./instances";
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

test("the_table_is_sorted_and_a_name_finds_its_figures_whatever_the_case", () => {
  const names = INSTANCE_TYPES.map((i) => i.name);
  expect(names).toEqual([...names].sort());
  const m5 = instanceByName(" AWS/M5.Large ");
  expect(m5 && instanceLine(m5)).toBe("2 vCPU, 8 GiB, up to 10 Gbit/s");
  expect(instanceByName("aws/m5.huge")).toBeUndefined();
});
