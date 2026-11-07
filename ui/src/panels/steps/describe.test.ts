import { expect, test } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import { anyChain, retryingCalls, routeKeys, stepLabel, stepsSummary } from "./describe";
import { readChain } from "./model";

const steps = (chain: unknown) => readChain(chain, [])!;
const docOf = (chains: Record<string, unknown>): Doc =>
  ({
    endpoints: Object.fromEntries(Object.entries(chains).map(([k, c]) => [k, { chain: c }])),
  }) as Doc;

test("a_step_row_says_the_callee_the_op_the_repeat_and_the_chance_when_they_differ_from_plain", () => {
  const [a, b, c, d, e] = steps([
    "db",
    { call: "db", op: "read", repeat: 2, prob: 0.8 },
    { call: "db", repeat: 1, prob: 1 },
    { cache: "c" },
    { parallel: ["a", ["b", "c"]] },
  ]);
  expect(stepLabel(a!)).toBe("call db");
  expect(stepLabel(b!)).toBe("call db · read · ×2 · 80%");
  expect(stepLabel(c!)).toBe("call db");
  expect(stepLabel(d!)).toBe("cache c");
  expect(stepLabel(e!)).toBe("parallel · 2 branches");
  expect(stepLabel(steps([{ x: 1 }])[0]!)).toBe("A step this panel cannot edit");
});

test("retries_are_counted_inside_parallel_branches_too", () => {
  const list = steps([
    { call: "a", retry: {} },
    { parallel: [[{ call: "b", retry: { attempts: 2 } }], "c"] },
  ]);
  expect(retryingCalls(list)).toBe(2);
});

test("the_summary_counts_routes_steps_and_own_retries_and_says_default_with_none", () => {
  expect(stepsSummary(docOf({}))).toBe("default chains");
  expect(stepsSummary(undefined)).toBe("default chains");
  expect(
    stepsSummary(
      docOf({ "GET /a": ["db", { call: "x", retry: {} }], "GET /b": [{ parallel: ["c", "d"] }] }),
    ),
  ).toBe("2 routes · 5 steps · 1 retry");
  expect(stepsSummary(docOf({ "GET /a": ["db"] }))).toBe("1 route · 1 step");
});

test("a_route_with_no_chain_key_is_a_default_one_and_the_group_is_unchanged", () => {
  const doc = { endpoints: { "GET /a": {}, "GET /b": { op: "read" } } } as Doc;
  expect(routeKeys(doc)).toEqual(["GET /a", "GET /b"]);
  expect(anyChain(doc)).toBe(false);
  expect(anyChain(docOf({ "GET /a": [] }))).toBe(true);
});

test("endpoints_given_as_a_file_path_have_no_routes_to_edit", () => {
  expect(routeKeys({ endpoints: "endpoints.yaml" } as Doc)).toEqual([]);
});
