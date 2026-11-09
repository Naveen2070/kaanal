import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useMode } from "../../data/store/mode";
import { useRun } from "../../data/store/run";
import { mockFetch } from "../../test/fetch";
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
  useMode.setState({ mode: "live" });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("studio_opens_live_and_the_switch_chooses_batch_where_run_appears", async () => {
  await show(300);
  expect(screen.getByRole("button", { name: "Live" }).getAttribute("aria-pressed")).toBe("true");
  expect(screen.queryByRole("button", { name: "Run" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Batch" }));
  expect(useMode.getState().mode).toBe("batch");
  expect(screen.getByRole("button", { name: "Run" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Live" }));
  expect(screen.queryByRole("button", { name: "Run" })).toBeNull();
});

test("the_run_button_runs", async () => {
  useMode.setState({ mode: "batch" });
  const onRun = await show(300);
  fireEvent.click(screen.getByRole("button", { name: "Run" }));
  expect(onRun).toHaveBeenCalledTimes(1);
});

test("a_blocked_run_says_so_in_the_status_line_and_the_reason_is_a_notice", async () => {
  useMode.setState({ mode: "batch" });
  await show(300);
  useRun.setState({ status: "blocked", lastError: "no entry node", dimmed: true });
  expect((await screen.findByRole("status")).textContent).toBe("Results out of date");
  expect(screen.queryByText("no entry node")).toBeNull(); // the reason is a toast (toast/bind.ts)
});

test("topbar_has_one_h1_with_the_product_name_and_the_project_as_its_scope", async () => {
  await show(300);
  const h1 = screen.getAllByRole("heading", { level: 1 });
  expect(h1).toHaveLength(1);
  expect(h1[0]?.textContent?.trim()).toBe("Kaanal");
});

test("there_is_one_status_line_for_results_and_draft", async () => {
  await show(300);
  expect(screen.getAllByRole("status")).toHaveLength(1);
  expect(screen.queryByText(/^Draft: /)).toBeNull();
});

test("the_h1_names_the_project_folder_when_there_is_one", async () => {
  mockFetch([{ json: { ...view(300), project_name: "petstore" } }]);
  await useDraft.getState().load();
  render(<TopBar onRun={vi.fn()} />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Kaanal / petstore");
});

test("a_pending_structure_edit_says_it_waits_for_apply_in_live_mode", async () => {
  const { useLive } = await import("../../data/store/live");
  await show(300);
  expect(screen.queryByText(/wait for Apply/)).toBeNull();
  useLive.getState().noteEdit({ live: "applied", changed: ["app"], structure_pending: true });
  expect(await screen.findByText(/wait for Apply/)).toBeTruthy();
  useLive.getState().noteEdit({ live: "unchanged", structure_pending: false });
  expect(await screen.findByRole("group", { name: "Mode" })).toBeTruthy();
  await vi.waitFor(() => expect(screen.queryByText(/wait for Apply/)).toBeNull());
  useLive.getState().noteEdit({ live: "none" });
});

test("the_admin_button_sets_the_mode_is_pressed_and_keeps_the_pending_structure_note", async () => {
  await show(100);
  const btn = screen.getByRole("button", { name: "Admin" });
  expect(btn.getAttribute("aria-pressed")).toBe("false");
  fireEvent.click(btn);
  expect(useMode.getState().mode).toBe("admin");
  expect(btn.getAttribute("aria-pressed")).toBe("true");
  expect(screen.getByRole("button", { name: "Live" }).getAttribute("aria-pressed")).toBe("false");
  // admin shows no Run button either: only Batch runs once
  expect(screen.queryByRole("button", { name: "Run" })).toBeNull();
  useMode.getState().set("live");
});
