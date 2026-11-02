// The palette: full list on a wide window, an icon rail with names between 900 and 1279 px.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { NODE_KINDS } from "./nodeKinds";
import { Palette } from "./Palette";
import { useNotes } from "./notes";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("a_wide_window_shows_each_kind_with_its_word", () => {
  vi.stubGlobal("innerWidth", 1440);
  render(<Palette onAdd={() => {}} />);
  for (const { label } of NODE_KINDS) {
    expect(screen.getByRole("button", { name: new RegExp(label) }).textContent).toContain(label);
  }
});

test("palette_collapses_to_an_icon_rail_with_names", () => {
  vi.stubGlobal("innerWidth", 1000);
  render(<Palette onAdd={() => {}} />);
  const buttons = screen.getAllByRole("button");
  expect(buttons).toHaveLength(NODE_KINDS.length);
  for (const [i, { label }] of NODE_KINDS.entries()) {
    expect(buttons[i]!.getAttribute("aria-label")).toBe(label); // an icon alone has no name
    expect(buttons[i]!.textContent).toBe(""); // and no visible word
  }
});

test("clicking_a_kind_adds_it_in_both_layouts", () => {
  for (const w of [1440, 1000]) {
    vi.stubGlobal("innerWidth", w);
    const onAdd = vi.fn();
    const { unmount } = render(<Palette onAdd={onAdd} />);
    fireEvent.click(screen.getAllByRole("button")[1]!);
    expect(onAdd).toHaveBeenCalledWith(NODE_KINDS[1]!.kind);
    unmount();
  }
});

test("the_wide_palette_says_how_to_move_around_and_the_icon_rail_has_no_room_for_it", () => {
  vi.stubGlobal("innerWidth", 1440);
  const { unmount } = render(<Palette onAdd={() => {}} />);
  const hint = screen.getByTestId("canvas-hint").textContent ?? "";
  expect(hint).toMatch(/pan/);
  expect(hint).toMatch(/scroll to zoom/);
  expect(hint).toMatch(/right edge/);
  unmount();
  vi.stubGlobal("innerWidth", 1000);
  render(<Palette onAdd={() => {}} />);
  expect(screen.queryByTestId("canvas-hint")).toBeNull();
});

test("add_a_note_pins_a_caption_on_the_wide_palette_only", () => {
  vi.stubGlobal("innerWidth", 1440);
  useNotes.setState({ project: null, notes: [] });
  const { unmount } = render(<Palette onAdd={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: /Add a note/ }));
  expect(useNotes.getState().notes).toHaveLength(1);
  unmount();
  vi.stubGlobal("innerWidth", 1000);
  render(<Palette onAdd={() => {}} />);
  expect(screen.queryByRole("button", { name: /Add a note/ })).toBeNull();
});
