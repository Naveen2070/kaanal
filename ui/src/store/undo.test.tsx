import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, Op } from "../api/types";
import { mockFetch } from "../test/fetch";
import { UndoRedo } from "../topbar/UndoRedo";
import { useDraft } from "./draft";
import { history, useHistory } from "./history";
import { applyOps } from "./optimistic";

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b1",
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service", concurrency: 4 } },
      edges: [{ from: "gw", to: "app" }],
    },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...over,
  };
}

/** The server's answer to `ops` on `v`: the draft with them applied. */
const after = (v: DraftView, ops: Op[]) => applyOps(v, ops);

async function loaded(v: DraftView) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
}

const edit: Op[] = [{ op: "set", path: ["nodes", "app", "concurrency"], value: 9 }];

beforeEach(() => {
  history.clear();
  useDraft.setState({ view: null, error: null, restarted: false });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("an_edit_is_recorded_and_undo_and_redo_send_the_inverse_and_the_forward", async () => {
  const v0 = view();
  await loaded(v0);
  const v1 = after(v0, edit);
  mockFetch([{ json: v1 }]);
  expect(await useDraft.getState().patch(edit)).toBe(true);
  expect(useHistory.getState().past).toHaveLength(1);

  const sent = mockFetch([{ json: v0 }, { json: v1 }]);
  expect(await useDraft.getState().undo()).toBe(true);
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency"], value: 4 }],
  });
  expect(useDraft.getState().view?.doc?.nodes?.app).toEqual({ type: "service", concurrency: 4 });
  expect(await useDraft.getState().redo()).toBe(true);
  expect(sent[1]?.body).toEqual({ patch: edit });
  expect(useHistory.getState().past).toHaveLength(1); // undo and redo are not new edits
});

test("a_refused_edit_is_not_recorded", async () => {
  await loaded(view());
  mockFetch([{ status: 400, json: { error: "no" } }]);
  expect(await useDraft.getState().patch(edit)).toBe(false);
  expect(useHistory.getState().past).toHaveLength(0);
});

test("an_outside_rev_change_clears_the_stack", async () => {
  const v0 = view();
  await loaded(v0);
  mockFetch([{ json: { ...after(v0, edit), rev: 1 } }]);
  await useDraft.getState().patch(edit);
  expect(useHistory.getState().past).toHaveLength(1);
  mockFetch([{ json: { ...v0, rev: 7 } }]); // someone else changed the file
  await useDraft.getState().load();
  expect(useHistory.getState().past).toHaveLength(0);
});

test("a_reload_that_changes_nothing_keeps_the_stack", async () => {
  const v0 = view();
  await loaded(v0);
  mockFetch([{ json: { ...after(v0, edit), rev: 1 } }]);
  await useDraft.getState().patch(edit);
  mockFetch([{ json: { ...after(v0, edit), rev: 1 } }]);
  await useDraft.getState().load();
  expect(useHistory.getState().past).toHaveLength(1);
});

test("a_stale_undo_the_draft_refuses_drops_the_history", async () => {
  const v0 = view();
  await loaded(v0);
  mockFetch([{ json: after(v0, edit) }]);
  await useDraft.getState().patch(edit);
  mockFetch([{ status: 400, json: { error: "gone" } }]);
  expect(await useDraft.getState().undo()).toBe(false);
  expect(useHistory.getState().past).toHaveLength(0);
  expect(useHistory.getState().future).toHaveLength(0);
});

test("a_connect_the_draft_takes_back_is_never_recorded", async () => {
  const v0 = view();
  await loaded(v0);
  const ops: Op[] = [{ op: "append", path: ["edges"], value: { from: "app", to: "app" } }];
  const v1 = after(v0, ops);
  mockFetch([
    { json: v1 },
    {
      json: {
        ok: false,
        diagnostics: [
          { file: null, line: 1, col: 1, path: ["edges", 1, "to"], message: "no self call" },
        ],
        warnings: [],
        flagged: [],
      },
    },
    { json: v0 },
  ]);
  const r = await useDraft.getState().connect("app", "app");
  expect(r.ok).toBe(false);
  expect(useHistory.getState().past).toHaveLength(0);
});

test("ctrl_z_and_ctrl_shift_z_work_outside_text_inputs_only", async () => {
  const v0 = view();
  await loaded(v0);
  mockFetch([{ json: after(v0, edit) }]);
  await useDraft.getState().patch(edit);
  const undo = vi.fn(async () => true);
  const redo = vi.fn(async () => true);
  useDraft.setState({ undo, redo });
  render(
    <>
      <UndoRedo />
      <input aria-label="field" />
    </>,
  );
  // in a text field the key is the field's own
  fireEvent.keyDown(screen.getByLabelText("field"), { key: "z", ctrlKey: true });
  expect(undo).not.toHaveBeenCalled();
  await act(async () => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
  expect(undo).toHaveBeenCalledTimes(1);
  await act(
    async () => void fireEvent.keyDown(document.body, { key: "Z", ctrlKey: true, shiftKey: true }),
  );
  expect(redo).toHaveBeenCalledTimes(1);
});

test("the_buttons_are_named_and_disabled_without_history", () => {
  render(<UndoRedo />);
  expect(screen.getByRole("button", { name: "Undo" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("button", { name: "Redo" }).hasAttribute("disabled")).toBe(true);
  act(() => history.push([], []));
  expect(screen.getByRole("button", { name: "Undo" }).hasAttribute("disabled")).toBe(false);
});
