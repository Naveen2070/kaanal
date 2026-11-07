import { expect, test } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import {
  addBranchOps,
  addToBranchOps,
  removeBranchOps,
  addStepOps,
  chainOf,
  moveOps,
  readChain,
  removeStepOps,
  setFieldOps,
  type CallStep,
  type ParallelStep,
} from "./model";

const KEY = "GET /orders";
const doc = (chain: unknown): Doc => ({ endpoints: { [KEY]: { chain } } }) as Doc;
const base = ["endpoints", KEY, "chain"];

test("a_bare_name_a_call_a_cache_and_a_parallel_are_read_as_what_they_are", () => {
  const steps = readChain(
    [
      "db",
      { call: "api", op: "read", repeat: 2, prob: 0.5, retry: { attempts: 3 } },
      { cache: "c" },
      { parallel: ["a", ["b", { call: "c" }]] },
    ],
    base,
  )!;
  expect(steps.map((s) => s.kind)).toEqual(["call", "call", "cache", "parallel"]);
  const [bare, call] = steps as CallStep[];
  expect(bare).toMatchObject({ node: "db", bare: true, path: [...base, 0] });
  expect(call).toMatchObject({ node: "api", op: "read", repeat: 2, prob: 0.5, bare: false });
  expect(call?.retry).toEqual({ attempts: 3 });
  const par = steps[3] as ParallelStep;
  expect(par.branches.map((b) => b.list)).toEqual([false, true]);
  expect(par.branches[1]?.steps.map((s) => s.path)).toEqual([
    [...base, 3, "parallel", 1, 0],
    [...base, 3, "parallel", 1, 1],
  ]);
  expect(par.branches[0]?.steps[0]?.path).toEqual([...base, 3, "parallel", 0]);
});

test("a_step_the_editor_does_not_understand_is_locked_not_dropped", () => {
  const steps = readChain([{ weird: 1 }, 7.5, null], base)!;
  expect(steps.map((s) => s.kind)).toEqual(["locked", "locked", "locked"]);
  // an integer is a node name (the loader reads `- 1` as one)
  expect(readChain([1], base)![0]).toMatchObject({ kind: "call", node: "1" });
});

test("a_call_keeps_the_keys_the_editor_does_not_edit", () => {
  const [s] = readChain(
    [{ call: "db", with: "w", via: "GET /x", chain: ["a"] }],
    base,
  ) as CallStep[];
  expect(s?.kept.sort()).toEqual(["chain", "via", "with"]);
});

test("a_chain_that_is_not_a_list_is_not_read", () => {
  expect(readChain("db", base)).toBeNull();
  expect(readChain(undefined, base)).toBeNull();
});

test("chain_of_finds_the_route_chain_or_says_there_is_none", () => {
  expect(chainOf(doc(["db"]), KEY)).toEqual(["db"]);
  expect(chainOf({ endpoints: { [KEY]: {} } } as Doc, KEY)).toBeUndefined();
  expect(chainOf({} as Doc, KEY)).toBeUndefined();
});

test("setting_a_field_on_a_map_step_sets_or_inserts_the_key", () => {
  const [s] = readChain([{ call: "db", repeat: 2 }], base) as CallStep[];
  expect(setFieldOps(s!, "repeat", 3)).toEqual([
    { op: "set", path: [...base, 0, "repeat"], value: 3 },
  ]);
  expect(setFieldOps(s!, "prob", 0.5)).toEqual([
    { op: "insert", path: [...base, 0, "prob"], value: 0.5 },
  ]);
});

test("clearing_a_field_removes_the_key_and_clearing_a_missing_one_does_nothing", () => {
  const [s] = readChain([{ call: "db", repeat: 2 }], base) as CallStep[];
  expect(setFieldOps(s!, "repeat", undefined)).toEqual([
    { op: "remove", path: [...base, 0, "repeat"] },
  ]);
  expect(setFieldOps(s!, "prob", undefined)).toEqual([]);
});

test("editing_a_bare_name_rewrites_it_as_a_map_with_the_call_key_first", () => {
  const [s] = readChain(["db"], base) as CallStep[];
  expect(setFieldOps(s!, "repeat", 2)).toEqual([
    { op: "set", path: [...base, 0], value: { call: "db", repeat: 2 } },
  ]);
  expect(setFieldOps(s!, "repeat", undefined)).toEqual([]);
});

test("adding_a_step_appends_to_the_chain_or_creates_the_chain", () => {
  expect(addStepOps(doc(["db"]), KEY, { call: "api" })).toEqual([
    { op: "append", path: base, value: { call: "api" } },
  ]);
  expect(addStepOps({ endpoints: { [KEY]: {} } } as Doc, KEY, "db")).toEqual([
    { op: "insert", path: base, value: ["db"] },
  ]);
  expect(addStepOps({ endpoints: {} } as Doc, KEY, "db")).toEqual([
    { op: "insert", path: ["endpoints", KEY], value: { chain: ["db"] } },
  ]);
  expect(addStepOps({} as Doc, KEY, "db")).toEqual([
    { op: "insert", path: ["endpoints"], value: { [KEY]: { chain: ["db"] } } },
  ]);
});

test("removing_a_step_removes_its_path", () => {
  const steps = readChain(["a", "b"], base)!;
  expect(removeStepOps(steps[1]!)).toEqual([{ op: "remove", path: [...base, 1] }]);
});

test("a_move_is_one_swap_of_two_neighbours_and_the_ends_cannot_move_out", () => {
  const steps = readChain(["a", { call: "b", repeat: 2 }, { cache: "c" }], base)!;
  expect(moveOps(steps, 1, -1)).toEqual([{ op: "swap", path: [...base, 1], with: [...base, 0] }]);
  expect(moveOps(steps, 0, 1)).toEqual([{ op: "swap", path: [...base, 0], with: [...base, 1] }]);
  expect(moveOps(steps, 0, -1)).toEqual([]);
  expect(moveOps(steps, 2, 1)).toEqual([]);
});

test("a_move_never_rewrites_the_steps_so_their_keys_and_comments_are_untouched", () => {
  const steps = readChain([{ call: "a", with: "w" }, "b"], base)!;
  const ops = moveOps(steps, 0, 1);
  expect(ops).toHaveLength(1);
  expect(JSON.stringify(ops)).not.toContain('"w"');
});

test("a_branch_knows_its_path_and_how_it_was_written", () => {
  const [p] = readChain([{ parallel: ["a", ["b", "c"]] }], base) as ParallelStep[];
  expect(p?.branches.map((b) => [b.list, b.path, b.raw])).toEqual([
    [false, [...base, 0, "parallel", 0], "a"],
    [true, [...base, 0, "parallel", 1], ["b", "c"]],
  ]);
});

test("adding_to_a_list_branch_appends_and_to_a_bare_branch_makes_it_a_list", () => {
  const [p] = readChain([{ parallel: ["a", ["b"]] }], base) as ParallelStep[];
  expect(addToBranchOps(p!.branches[1]!, "x")).toEqual([
    { op: "append", path: [...base, 0, "parallel", 1], value: "x" },
  ]);
  expect(addToBranchOps(p!.branches[0]!, { call: "x" })).toEqual([
    { op: "set", path: [...base, 0, "parallel", 0], value: ["a", { call: "x" }] },
  ]);
});

test("a_branch_is_added_at_the_end_and_the_last_branch_cannot_be_removed", () => {
  const [p] = readChain([{ parallel: ["a", "b"] }], base) as ParallelStep[];
  expect(addBranchOps(p!, "c")).toEqual([
    { op: "append", path: [...base, 0, "parallel"], value: "c" },
  ]);
  expect(removeBranchOps(p!, 1)).toEqual([{ op: "remove", path: [...base, 0, "parallel", 1] }]);
  const [one] = readChain([{ parallel: ["a"] }], base) as ParallelStep[];
  expect(removeBranchOps(one!, 0)).toEqual([]);
});
