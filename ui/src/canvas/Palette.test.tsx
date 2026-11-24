// The palette: full list on a wide window, an icon rail with names between 900 and 1279 px.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../data/store/draft";
import { NODE_KINDS } from "./nodeKinds";
import { Palette } from "./Palette";
import { useGuide } from "../overlays/guide/store";
import { notesOf, seedDraft } from "../test/notesDraft";

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
  // every kind, then the note
  expect(buttons).toHaveLength(NODE_KINDS.length + 1);
  for (const [i, { label }] of NODE_KINDS.entries()) {
    expect(buttons[i]!.getAttribute("aria-label")).toBe(label); // an icon alone has no name
    expect(buttons[i]!.textContent).toBe(""); // and no visible word
  }
});

test("clicking_a_kind_adds_it_in_both_layouts", () => {
  for (const w of [1440, 1366, 1000]) {
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
  expect(hint.length).toBeLessThan(100);
  unmount();
  vi.stubGlobal("innerWidth", 1000);
  render(<Palette onAdd={() => {}} />);
  expect(screen.queryByTestId("canvas-hint")).toBeNull();
});

test("the_icon_rail_starts_below_1400_px_so_a_laptop_gets_it", () => {
  for (const [w, rail] of [
    [1399, true],
    [1366, true],
    [1400, false],
  ] as const) {
    vi.stubGlobal("innerWidth", w);
    const { unmount } = render(<Palette onAdd={() => {}} />);
    expect(screen.queryByTestId("canvas-hint") === null, `${w}`).toBe(rail);
    unmount();
  }
});

test("an_icon_has_a_card_with_the_description_and_a_glossary_link", () => {
  vi.stubGlobal("innerWidth", 1366);
  render(<Palette onAdd={() => {}} />);
  const cache = screen.getByRole("button", { name: "Cache" });
  // the card is the icon's description, so assistive technology reads it too
  const card = document.getElementById(cache.getAttribute("aria-describedby") ?? "");
  expect(card?.getAttribute("role")).toBe("tooltip");
  expect(card?.textContent).toContain("Answers repeat reads fast");
  fireEvent.click(screen.getAllByRole("link", { name: "Open in the glossary" })[4] as HTMLElement);
  expect(useGuide.getState()).toMatchObject({ open: true, entry: "cache" });
  useGuide.getState().hide();
});

test("add_a_note_pins_a_caption_in_both_layouts", async () => {
  vi.stubGlobal("innerWidth", 1440);
  await seedDraft();
  vi.stubGlobal("innerWidth", 1440);
  const { unmount } = render(<Palette onAdd={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: /Add a note/ }));
  expect(notesOf()).toHaveLength(1);
  unmount();
  await useDraft.getState().settled();
  await seedDraft();
  vi.stubGlobal("innerWidth", 1000);
  render(<Palette onAdd={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "Add a note" }));
  expect(notesOf()).toHaveLength(1);
});

test("storage_is_offered_with_its_one_line_description", () => {
  vi.stubGlobal("innerWidth", 1440);
  const onAdd = vi.fn();
  render(<Palette onAdd={onAdd} />);
  const button = screen.getByRole("button", { name: /Storage/ });
  expect(button.title).toContain("Object storage like S3");
  fireEvent.click(button);
  expect(onAdd).toHaveBeenCalledWith("storage");
});

test("the_palette_offers_the_kinds_the_server_lists_and_a_plain_box_for_an_unknown_one", () => {
  useDraft.setState({
    view: {
      kinds: [
        { kind: "storage", callees: [] },
        { kind: "queue", callees: [] },
      ],
    },
  } as never);
  render(<Palette onAdd={vi.fn()} />);
  expect(screen.getByRole("button", { name: /Storage/ })).toBeTruthy();
  expect(screen.getByRole("button", { name: /queue/ })).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Gateway/ })).toBeNull();
  useDraft.setState({ view: null });
});
