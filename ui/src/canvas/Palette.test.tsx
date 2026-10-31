// The palette: full list on a wide window, an icon rail with names between 900 and 1279 px.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { NODE_KINDS } from "./nodeKinds";
import { Palette } from "./Palette";

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
