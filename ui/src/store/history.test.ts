import { beforeEach, expect, test } from "vite-plus/test";
import type { DraftView, Op } from "../api/types";
import { HISTORY_LIMIT, history, invertOps, useHistory } from "./history";
import { applyOps } from "./optimistic";

const view = (): DraftView => ({
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: { app: { type: "service", concurrency: 4 }, db: { type: "database" } },
    edges: [
      { from: "app", to: "db" },
      { from: "db", to: "app" },
    ],
  },
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: { service: { type: "service" } },
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
});

beforeEach(() => history.clear());

/** `ops` then their inverse leaves the doc as it was. */
function roundTrip(ops: Op[]) {
  const v = view();
  const inv = invertOps(v, ops);
  expect(inv).not.toBeNull();
  const after = applyOps(v, ops);
  expect(after.doc).not.toEqual(v.doc);
  expect(applyOps(after, inv as Op[]).doc).toEqual(v.doc);
}

test("undo_applies_the_inverse_patch_and_redo_the_forward", () => {
  history.push([{ op: "remove", path: ["a"] }], [{ op: "set", path: ["a"], value: 1 }]);
  expect(history.undo()).toEqual([{ op: "remove", path: ["a"] }]);
  expect(useHistory.getState().future).toHaveLength(1);
  expect(history.redo()).toEqual([{ op: "set", path: ["a"], value: 1 }]);
  expect(history.redo()).toBeNull();
  history.undo();
  history.push([], []); // a new edit ends the redo line
  expect(history.redo()).toBeNull();
});

test("an_outside_change_clears_the_stack", () => {
  history.push([], []);
  history.clear();
  expect(history.undo()).toBeNull();
});

test("a_rename_is_inverted_by_a_rename_back", () => {
  expect(invertOps(view(), [{ op: "rename_node", from: "app", to: "api" }])).toEqual([
    { op: "rename_node", from: "api", to: "app" },
  ]);
  roundTrip([{ op: "rename_node", from: "app", to: "api" }]);
});

test("set_insert_append_remove_and_add_node_all_invert", () => {
  roundTrip([{ op: "set", path: ["nodes", "app", "concurrency"], value: 9 }]);
  roundTrip([{ op: "insert", path: ["nodes", "app", "timeout"], value: "1s" }]);
  roundTrip([{ op: "append", path: ["edges"], value: { from: "app", to: "app" } }]);
  roundTrip([{ op: "remove", path: ["edges", 0] }]);
  roundTrip([{ op: "remove", path: ["nodes", "db"] }]);
  roundTrip([{ op: "add_node", name: "service-1", kind: "service" }]);
  // a delete is several ops at once
  roundTrip([
    { op: "remove", path: ["edges", 1] },
    { op: "remove", path: ["edges", 0] },
    { op: "remove", path: ["nodes", "db"] },
  ]);
});

test("an_edit_that_cannot_be_inverted_is_not_recorded_and_says_so", () => {
  expect(invertOps(view(), [{ op: "append", path: ["nodes"], value: 1 }])).toBeNull(); // not a list
  expect(invertOps(view(), [{ op: "remove", path: ["nodes", "ghost"] }])).toBeNull();
  expect(invertOps({ ...view(), doc: null }, [{ op: "set", path: ["x"], value: 1 }])).toBeNull();
  history.skip();
  expect(useHistory.getState().note).toMatch(/can't be undone/);
  history.push([], []);
  expect(useHistory.getState().note).toBeNull();
});

test("stack_is_bounded_at_100", () => {
  for (let i = 0; i < HISTORY_LIMIT + 20; i++) {
    history.push([{ op: "remove", path: [i] }], []);
  }
  expect(useHistory.getState().past).toHaveLength(HISTORY_LIMIT);
  // the oldest went, the newest stayed
  expect(history.undo()).toEqual([{ op: "remove", path: [HISTORY_LIMIT + 19] }]);
});
