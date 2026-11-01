import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { expectNoViolations } from "../test/axe";
import { ControlDock } from "./ControlDock";

// the selection normally lives in the address bar (nuqs); here it is a plain variable
const sel = vi.hoisted(() => ({ name: null as string | null, set: vi.fn() }));
vi.mock("../store/view", () => ({ useSelected: () => [sel.name, sel.set] }));

const view: DraftView = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: { nodes: { "api #1": { type: "service", concurrency: 8, proc: "20ms" } }, edges: [] },
  doc_error: null,
  project_id: "none",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
};

beforeAll(() => {
  // what Radix Slider needs and jsdom lacks
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  Object.assign(Element.prototype, {
    setPointerCapture() {},
    releasePointerCapture() {},
    hasPointerCapture: () => false,
  });
});

function show(name: string | null) {
  sel.name = name;
  useDraft.setState({ view, error: null });
  return render(<ControlDock />);
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null });
  sel.set.mockClear();
});
afterEach(cleanup);

test("nothing_is_selected_so_there_is_no_dock", () => {
  show(null);
  expect(screen.queryByRole("region")).toBeNull();
  expect(screen.queryByRole("slider")).toBeNull();
});

test("a_selected_component_opens_the_dock_with_its_name_kind_and_rows", () => {
  show("api #1");
  const dock = screen.getByRole("region", { name: "Settings for api #1" });
  expect(dock.textContent).toContain("Service");
  for (const row of ["Time", "Slots", "Copies"]) {
    expect(screen.getByRole("slider", { name: row })).toBeTruthy();
  }
});

test("the_close_button_clears_the_selection", () => {
  show("api #1");
  fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
  expect(sel.set).toHaveBeenCalledWith(null);
});

test("escape_in_the_dock_clears_the_selection", () => {
  show("api #1");
  fireEvent.keyDown(screen.getByRole("slider", { name: "Slots" }), { key: "Escape" });
  expect(sel.set).toHaveBeenCalledWith(null);
});

test("a_component_that_is_gone_closes_the_dock", () => {
  show("ghost");
  expect(screen.queryByRole("region")).toBeNull();
});

test("the_dock_has_no_axe_violations", async () => {
  show("api #1");
  await expectNoViolations(document.body);
});
