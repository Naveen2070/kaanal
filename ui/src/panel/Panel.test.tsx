import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { mockFetch } from "../test/fetch";
import { Panel } from "./Panel";

const view: DraftView = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: { nodes: { app: { type: "service", concurrency: 4 } }, edges: [] },
  doc_error: null,
  project_id: "none",
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
};

async function show(url: string) {
  history.replaceState(null, "", url);
  mockFetch([{ json: view }, { json: { nodes: [], edges: [] } }]);
  await useDraft.getState().load();
  render(
    <NuqsAdapter>
      <Panel />
    </NuqsAdapter>,
  );
}

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("nothing_selected_asks_for_a_selection", async () => {
  await show("/__kaanal/studio");
  expect(screen.getByText(/Select a component/)).toBeTruthy();
});

test("a_selected_node_shows_its_controls_and_a_missing_one_asks_again", async () => {
  await show("/__kaanal/studio?node=app");
  expect(screen.getByLabelText("Slots per instance")).toBeTruthy();
  cleanup();
  await show("/__kaanal/studio?node=gone");
  expect(screen.getByText(/Select a component/)).toBeTruthy();
});

test("the_tabs_switch_between_controls_log_and_results", async () => {
  await show("/__kaanal/studio?node=app");
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Log" })));
  expect(await screen.findByText(/No finished run yet. The log appears/)).toBeTruthy();
  expect(screen.getByRole("tab", { name: "Log" }).getAttribute("aria-selected")).toBe("true");
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Results" })));
  expect(await screen.findByText(/The results appear after the first run/)).toBeTruthy();
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Controls" })));
  expect(await screen.findByLabelText("Slots per instance")).toBeTruthy();
});
