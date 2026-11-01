import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { HELP, HELP_LABEL, type HelpId } from "./copy";
import { HelpPopover } from "./HelpPopover";

afterEach(cleanup);

const ids = Object.keys(HELP) as HelpId[];

test("every_jargon_label_has_a_help_entry_under_25_words", () => {
  expect(ids.sort()).toEqual(
    ["copies", "goodput", "p99", "queue", "saturation", "slots", "time"].sort(),
  );
  for (const id of ids) {
    expect(HELP[id].split(/\s+/).length, id).toBeLessThan(25);
    expect(HELP_LABEL[id], id).toBeTruthy();
  }
});

test("popover_opens_on_click_and_closes_on_escape_with_focus_back", async () => {
  render(<HelpPopover id="p99" />);
  const trigger = screen.getByRole("button", { name: "About p99" });
  expect(trigger.tagName).toBe("BUTTON"); // a native button: Enter and Space open it
  fireEvent.click(trigger);
  expect(await screen.findByText(HELP.p99)).toBeTruthy();
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(screen.queryByText(HELP.p99)).toBeNull();
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
  await screen.findByText(HELP.slots);
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(seen).not.toHaveBeenCalled();
  window.removeEventListener("keydown", seen);
  fireEvent.click(screen.getByRole("button", { name: "About Slots" }));
  await screen.findByText(HELP.slots);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByText(HELP.slots)).toBeNull();
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
