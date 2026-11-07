import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { Glossary } from "./Glossary";
import { GuideHost } from "./GuideHost";
import { useGuide } from "./store";

beforeEach(() => useGuide.setState({ open: true, chapter: "components" }));
afterEach(cleanup);

test("opens_on_the_components_chapter_with_every_chapter_as_a_tab", () => {
  render(<Glossary />);
  const tabs = screen.getAllByRole("tab").map((t) => t.textContent);
  expect(tabs).toEqual([
    "Components",
    "Reading the screen",
    "Terms",
    "Wires and routes",
    "Wire settings",
    "Reading Results",
    "Keys",
  ]);
  expect(screen.getByRole("tab", { name: "Components" }).getAttribute("aria-selected")).toBe(
    "true",
  );
  // a component says what it is in a real system
  expect(screen.getByText(/nginx, Envoy, Kong/)).toBeTruthy();
});

test("choosing_a_tab_shows_that_chapter", () => {
  render(<Glossary />);
  fireEvent.mouseDown(screen.getByRole("tab", { name: "Reading the screen" }), { button: 0 });
  expect(useGuide.getState().chapter).toBe("screen");
  expect(screen.getByRole("heading", { name: "Reading the screen" })).toBeTruthy();
  expect(screen.getAllByText("Slowest work under").length).toBeGreaterThan(0);
});

test("search_looks_across_chapters_and_says_when_nothing_matches", () => {
  render(<Glossary />);
  const box = screen.getByRole("searchbox", { name: "Search the glossary" });
  fireEvent.change(box, { target: { value: "turned away" } });
  expect(screen.getAllByText(/Turned away|Turning requests away/).length).toBeGreaterThan(0);
  fireEvent.change(box, { target: { value: "zzzzqq" } });
  expect(screen.getByText("Nothing matches.")).toBeTruthy();
});

test("escape_and_the_close_button_close_it", async () => {
  const { unmount } = render(<GuideHost />);
  const dialog = await screen.findByRole("dialog", { name: "Glossary", hidden: true });
  act(() => {
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
  });
  expect(useGuide.getState().open).toBe(false);
  unmount();
  useGuide.setState({ open: true });
  render(<Glossary />);
  fireEvent.click(screen.getByRole("button", { name: "Close the glossary" }));
  expect(useGuide.getState().open).toBe(false);
});

test("components_show_their_own_icon_tile_in_their_hue", () => {
  render(<Glossary />);
  const gateway = screen.getByRole("figure", { name: "Gateway as it looks on the canvas" });
  const tile = gateway.querySelector("span[style]") as HTMLElement;
  // the tile is tinted from the kind's hue, as on the canvas
  expect(tile.getAttribute("style")).toContain("var(--cat-gateway)");
  expect(screen.queryByText("Example figures")).toBeNull(); // a plain tile shows a fact, not figures
});

test("reading_the_screen_shows_a_card_and_meters_drawn_with_the_real_meter", () => {
  useGuide.setState({ open: true, chapter: "screen" });
  render(<Glossary />);
  expect(screen.getByRole("figure", { name: /card named shop: 83% busy/ })).toBeTruthy();
  // the busy entry shows the three levels with their words
  const meters = screen.getByRole("figure", { name: /45% busy.*83% busy.*95% busy/ });
  expect(meters).toBeTruthy();
  expect(document.querySelectorAll("meter").length).toBeGreaterThan(3);
});

test("an_entry_asked_for_opens_its_chapter_and_is_marked", () => {
  useGuide.setState({ open: true, chapter: "components", entry: "p99" });
  render(<Glossary />);
  expect(
    screen.getByRole("tab", { name: "Reading the screen" }).getAttribute("aria-selected"),
  ).toBe("true");
  const row = document.getElementById("glossary-p99");
  expect(row?.getAttribute("data-marked")).toBe("true");
});

test("an_entry_that_does_not_exist_leaves_the_chapter_alone", () => {
  useGuide.setState({ open: true, chapter: "components", entry: "no-such-entry" });
  render(<Glossary />);
  expect(screen.getByRole("tab", { name: "Components" }).getAttribute("aria-selected")).toBe(
    "true",
  );
  expect(document.querySelector("[data-marked]")).toBeNull();
});

test("choosing_a_chapter_clears_the_mark", () => {
  useGuide.setState({ open: true, chapter: "components", entry: "p99" });
  render(<Glossary />);
  fireEvent.mouseDown(screen.getByRole("tab", { name: "Components" }), { button: 0 });
  expect(useGuide.getState().entry).toBeNull();
  expect(document.querySelector("[data-marked]")).toBeNull();
});
