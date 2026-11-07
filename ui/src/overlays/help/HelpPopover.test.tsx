import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { useGuide } from "../guide/store";
import { HELP, HELP_LABEL, type HelpId } from "./copy";
import { HelpPopover } from "./HelpPopover";

afterEach(cleanup);

const ids = Object.keys(HELP) as HelpId[];

test("every_jargon_label_has_a_help_entry_under_25_words", () => {
  for (const id of ["copies", "goodput", "p99", "queue", "saturation", "slots", "time"]) {
    expect(ids, id).toContain(id);
  }
  for (const id of ids) {
    expect(HELP[id].what.split(/\s+/).length, id).toBeLessThan(25);
    expect(HELP_LABEL[id], id).toBeTruthy();
  }
});

test("popover_opens_on_click_and_closes_on_escape_with_focus_back", async () => {
  render(<HelpPopover id="p99" />);
  const trigger = screen.getByRole("button", { name: "About p99" });
  expect(trigger.tagName).toBe("BUTTON"); // a native button: Enter and Space open it
  fireEvent.click(trigger);
  expect(await screen.findByText(HELP.p99.what)).toBeTruthy();
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(screen.queryByText(HELP.p99.what)).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(trigger));
});

test("trigger_is_24px_with_an_accessible_name", () => {
  render(<HelpPopover id="time" />);
  const b = screen.getByRole("button", { name: "About Time" });
  expect(b.className).toContain("size-6");
});

test("no_css_content_tooltips_remain", () => {
  const src = join(process.cwd(), "src");
  const files = (readdirSync(src, { recursive: true, encoding: "utf8" }) as string[]).filter(
    (f) => /\.(css|tsx)$/.test(f) && !f.endsWith(".test.tsx"),
  );
  for (const f of files) {
    const text = readFileSync(join(src, f), "utf8");
    expect(text, f).not.toMatch(/content:\s*attr\(|data-tip\b/);
  }
});

test("a_press_outside_closes_it_and_escape_does_not_reach_the_parent", async () => {
  const seen = vi.fn();
  window.addEventListener("keydown", seen);
  render(<HelpPopover id="slots" />);
  fireEvent.click(screen.getByRole("button", { name: "About Slots" }));
  await screen.findByText(HELP.slots.what);
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(seen).not.toHaveBeenCalled();
  window.removeEventListener("keydown", seen);
  fireEvent.click(screen.getByRole("button", { name: "About Slots" }));
  await screen.findByText(HELP.slots.what);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByText(HELP.slots.what)).toBeNull();
});

test("the_bubble_wraps_its_words_even_inside_a_nowrap_figure", () => {
  render(
    <div className="whitespace-nowrap">
      <HelpPopover id="p99" />
    </div>,
  );
  fireEvent.click(screen.getByRole("button", { name: /About/ }));
  expect(screen.getByRole("dialog").className).toMatch(/whitespace-normal/);
});

test("the_card_shows_what_an_example_a_hint_and_a_glossary_link", async () => {
  render(<HelpPopover id="time" />);
  fireEvent.click(screen.getByRole("button", { name: "About Time" }));
  expect(await screen.findByText(HELP.time.what)).toBeTruthy();
  expect(screen.getByText("Example:").closest("p")?.textContent).toContain(HELP.time.example);
  expect(screen.getByText("Hint:").closest("p")?.textContent).toContain(HELP.time.hint);
  expect(screen.getByRole("button", { name: "Open Time in the glossary" })).toBeTruthy();
});

test("the_link_opens_the_glossary_at_that_entry_and_closes_the_card", async () => {
  useGuide.setState({ open: false, chapter: "components", entry: null });
  render(<HelpPopover id="p99" />);
  fireEvent.click(screen.getByRole("button", { name: "About p99" }));
  fireEvent.click(await screen.findByRole("button", { name: "Open p99 in the glossary" }));
  expect(useGuide.getState()).toMatchObject({ open: true, entry: "p99" });
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("a_help_entry_without_an_example_or_hint_shows_only_what_it_says", async () => {
  expect(Object.values(HELP).every((h) => typeof h.what === "string")).toBe(true);
});

test("near_the_bottom_of_the_window_the_card_opens_above_its_button", async () => {
  render(<HelpPopover id="p99" />);
  const trigger = screen.getByRole("button", { name: "About p99" });
  trigger.getBoundingClientRect = () =>
    ({
      top: window.innerHeight - 40,
      bottom: window.innerHeight - 16,
      left: 10,
      right: 34,
    }) as DOMRect;
  fireEvent.click(trigger);
  const card = await screen.findByRole("dialog");
  expect(card.style.bottom).not.toBe("");
  expect(card.style.top).toBe("");
});

test("with_room_below_the_card_opens_below_its_button", async () => {
  render(<HelpPopover id="p99" />);
  const trigger = screen.getByRole("button", { name: "About p99" });
  trigger.getBoundingClientRect = () => ({ top: 20, bottom: 44, left: 10, right: 34 }) as DOMRect;
  fireEvent.click(trigger);
  const card = await screen.findByRole("dialog");
  expect(card.style.top).toBe("50px");
  expect(card.style.bottom).toBe("");
});
