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
  expect(tabs).toEqual(["Components", "Reading the screen", "Terms", "Wires and routes", "Keys"]);
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
  expect(screen.getByText("Slowest work under")).toBeTruthy();
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
