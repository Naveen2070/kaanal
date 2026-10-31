// The shell (spec 3, 6): breakpoints, the slots, and the read-only list below 900 px.

import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { ReadonlyNodes } from "./ReadonlyNodes";
import { Shell } from "./Shell";
import { layoutFor } from "./useLayout";

function width(px: number) {
  vi.stubGlobal("innerWidth", px);
}

const slots = {
  top: <header>top</header>,
  palette: <aside>palette</aside>,
  canvas: <div>canvas</div>,
  rail: <aside>rail</aside>,
  readonly: <p>readonly list</p>,
};

beforeEach(() => width(1440));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("layout_breakpoints", () => {
  expect(layoutFor(1280)).toBe("full");
  expect(layoutFor(1279)).toBe("rail");
  expect(layoutFor(900)).toBe("rail");
  expect(layoutFor(899)).toBe("readonly");
});

test("a_wide_window_shows_every_slot_and_not_the_read_only_list", () => {
  render(<Shell {...slots} />);
  for (const t of ["top", "palette", "canvas", "rail"]) expect(screen.getByText(t)).toBeTruthy();
  expect(screen.queryByText("readonly list")).toBeNull();
});

test("below_900_the_canvas_palette_and_rail_give_way_to_the_list", () => {
  width(899);
  render(<Shell {...slots} />);
  expect(screen.getByText("top")).toBeTruthy();
  expect(screen.getByText("readonly list")).toBeTruthy();
  for (const t of ["palette", "canvas", "rail"]) expect(screen.queryByText(t)).toBeNull();
});

test("resizing_across_a_breakpoint_changes_the_layout", () => {
  render(<Shell {...slots} />);
  expect(screen.getByText("canvas")).toBeTruthy();
  act(() => {
    width(700);
    window.dispatchEvent(new Event("resize"));
  });
  expect(screen.queryByText("canvas")).toBeNull();
  expect(screen.getByText("readonly list")).toBeTruthy();
});

const view: DraftView = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: {
      "front door": { type: "gateway" },
      "db #1/ü": { type: "database" },
    },
    edges: [],
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

test("readonly_below_900_lists_nodes_and_says_why", () => {
  useDraft.setState({ view });
  render(<ReadonlyNodes />);
  const items = screen.getAllByRole("listitem").map((li) => li.textContent);
  expect(items).toEqual([
    expect.stringContaining("front door"),
    expect.stringContaining("db #1/ü"),
  ]);
  expect(items[0]).toMatch(/gateway/i);
  expect(screen.getByRole("status").textContent).toMatch(/900/);
  expect(screen.getByRole("status").textContent).toMatch(/read-only|editing/i);
});

test("an_empty_draft_says_there_are_no_nodes_yet", () => {
  useDraft.setState({ view: { ...view, doc: { nodes: {}, edges: [] } } });
  render(<ReadonlyNodes />);
  expect(screen.queryAllByRole("listitem")).toHaveLength(0);
  expect(screen.getByText(/no components yet/i)).toBeTruthy();
});
