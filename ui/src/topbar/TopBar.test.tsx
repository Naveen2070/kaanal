import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { mockFetch } from "../test/fetch";
import { TopBar } from "./TopBar";

function view(rps: number | null): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {}, ...(rps === null ? {} : { traffic: { peak: { rps, journeys: [] } } }) },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

async function show(rps: number | null) {
  mockFetch([{ json: view(rps) }]);
  await useDraft.getState().load();
  const onRun = vi.fn();
  render(<TopBar onRun={onRun} />);
  return onRun;
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("the_run_button_runs", async () => {
  const onRun = await show(300);
  fireEvent.click(screen.getByRole("button", { name: "Run" }));
  expect(onRun).toHaveBeenCalledTimes(1);
});

test("a_blocked_run_says_so_in_the_status_line_and_the_reason_is_a_notice", async () => {
  await show(300);
  useRun.setState({ status: "blocked", lastError: "no entry node", dimmed: true });
  expect((await screen.findByRole("status")).textContent).toBe("Not up to date");
  expect(screen.queryByText("no entry node")).toBeNull(); // the reason is a toast (toast/bind.ts)
});

test("topbar_has_one_h1_with_the_project_name", async () => {
  await show(300);
  const h1 = screen.getAllByRole("heading", { level: 1 });
  expect(h1).toHaveLength(1);
  expect(h1[0]?.textContent).toBe("Kaanal Studio");
});

test("freshness_stamp_and_draft_state_are_separate_texts", async () => {
  await show(300);
  const draft = screen.getByText(/^Draft: /);
  const stamp = screen.getByRole("status");
  expect(draft).not.toBe(stamp);
  expect(draft.contains(stamp) || stamp.contains(draft)).toBe(false);
  expect(stamp.textContent).not.toMatch(/Draft/);
  expect(draft.textContent).toBe("Draft: applied · saved");
});

test("the_h1_names_the_project_folder_when_there_is_one", async () => {
  mockFetch([{ json: { ...view(300), project_name: "petstore" } }]);
  await useDraft.getState().load();
  render(<TopBar onRun={vi.fn()} />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("petstore");
});
