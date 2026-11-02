import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { useRail } from "../../shell/layout/rail";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { usePickedWire } from "../../canvas/picked";
import { mockFetch } from "../../test/fetch";
import { Panel } from "./Panel";

const view: DraftView = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: { app: { type: "service", concurrency: 4 }, db: { type: "database" } },
    edges: [{ from: "app", to: "db" }],
  },
  doc_error: null,
  project_id: "none",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
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

beforeEach(() => {
  useRail.setState({ open: true });
  useDraft.setState({ view: null, error: null, restarted: false });
  usePickedWire.setState({ index: null });
});
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
  expect(screen.getByLabelText("Slots")).toBeTruthy();
  cleanup();
  await show("/__kaanal/studio?node=gone");
  expect(screen.getByText(/Select a component/)).toBeTruthy();
});

test("the_tabs_switch_between_controls_log_and_results", async () => {
  await show("/__kaanal/studio?node=app");
  await act(async () => fireEvent.mouseDown(screen.getByRole("tab", { name: "Log" })));
  expect(await screen.findByText(/No finished run yet. The log appears/)).toBeTruthy();
  expect(screen.getByRole("tab", { name: "Log" }).getAttribute("aria-selected")).toBe("true");
  await act(async () => fireEvent.mouseDown(screen.getByRole("tab", { name: "Results" })));
  expect(await screen.findByText(/The results appear after the first run/)).toBeTruthy();
  await act(async () => fireEvent.mouseDown(screen.getByRole("tab", { name: "Controls" })));
  expect(await screen.findByLabelText("Slots")).toBeTruthy();
});

test("a_picked_wire_shows_its_settings_and_a_selected_component_wins", async () => {
  usePickedWire.setState({ index: 0 });
  await show("/__kaanal/studio");
  expect(screen.getByLabelText("Network latency (ms)")).toBeTruthy();
  expect(screen.getByTestId("rail-sentence").textContent).toBe("Wire from app to db");
  cleanup();
  await show("/__kaanal/studio?node=app");
  expect(screen.getByLabelText("Slots")).toBeTruthy();
  expect(screen.queryByLabelText("Network latency (ms)")).toBeNull();
  cleanup();
  usePickedWire.setState({ index: 7 }); // a place that is not there any more
  await show("/__kaanal/studio");
  expect(screen.getByText(/Select a component/)).toBeTruthy();
});
