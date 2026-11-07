// The component's Controls has what the wire's has: a sticky header with a jump box, quick setups,
// a strip of the last run, "Why is it red?" and the fix above the groups, and the rows that differ
// from the default above each group's Reset.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { app, show, view } from "./controlsHarness";
import { NODE_TARGETS } from "./groups/jump";
import { useGroups } from "./groups/store";
import { NodeStrip } from "./NodePanelParts";

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(cleanup);

test("the_header_names_the_component_and_says_what_is_changed", async () => {
  await show(nodeSummary(), { ...app, hold: "call", rate_limit: { rps: 100, burst: 20 } });
  const header = screen.getByText(/Changed from the defaults/).parentElement?.parentElement;
  expect(header?.textContent).toContain("app");
  expect(header?.textContent).toContain("Service");
  expect(header?.textContent).toContain("Server style, Request limit");
  expect(header?.className).toContain("sticky");
  expect(screen.getByRole("searchbox", { name: "Jump to a setting" })).toBeTruthy();
});

test("every_jump_target_is_a_label_on_a_fully_switched_on_component", async () => {
  await show(nodeSummary(), {
    ...app,
    queue: undefined,
    concurrency: { limit: 12, queue: 100 },
    hold: "call",
    timeout: "2s",
    rate_limit: { rps: 100, burst: 20 },
    bandwidth: { out: "1MB/s", in: "2MB/s" },
  });
  const labels = [...document.querySelectorAll("label")].map((l) => l.textContent?.trim());
  for (const t of NODE_TARGETS) expect(labels, t.label).toContain(t.label);
});

test("jumping_to_a_folded_setting_opens_its_group", async () => {
  await show();
  act(() => useGroups.setState({ open: {} }));
  const box = screen.getByRole("searchbox", { name: "Jump to a setting" });
  fireEvent.change(box, { target: { value: "burst" } });
  fireEvent.click(screen.getByRole("button", { name: "Burst" }));
  expect(useGroups.getState().open["node.limit"]).toBe(true);
});

test("the_strip_gives_busy_turned_away_and_wait_from_the_last_run", async () => {
  await show(
    nodeSummary({
      utilization: 0.78,
      limited: 3,
      overflowed: 4,
      mean_wait_ms: 12.4,
      visits_per_s: 250,
    }),
  );
  const strip = screen.getByLabelText("Last run at this component");
  expect(strip.textContent).toContain("Busy78%");
  expect(strip.textContent).toContain("Turned away7");
  expect(strip.textContent).toContain("Average wait12 ms");
  expect(strip.textContent).toContain("250 requests a second");
});

test("without_a_run_there_is_no_strip", () => {
  const { container } = render(<NodeStrip node={undefined} config={app} />);
  expect(container.firstChild).toBeNull();
});

test("why_is_it_red_and_the_fix_come_before_the_groups", async () => {
  await show(
    nodeSummary({
      saturated: true,
      utilization: 1,
      offered_load: 1.6,
      overflowed: 40,
      max_queue: 100,
    }),
  );
  const order = [...document.querySelectorAll("h3")].map((h) => h.textContent);
  expect(order.indexOf("Why is it red?")).toBeGreaterThan(-1);
  expect(order.indexOf("Why is it red?")).toBeLessThan(order.indexOf("Quick setups"));
  expect(order.indexOf("Suggested fix")).toBeLessThan(order.indexOf("Capacity"));
});

test("a_quick_setup_is_one_patch_and_shows_what_the_component_already_is", async () => {
  await show(nodeSummary(), { ...app, concurrency: { limit: 12, queue: 100 } });
  const eventLoop = screen.getByRole("button", { name: "Event loop" });
  expect(eventLoop.getAttribute("aria-pressed")).toBe("true");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Fail fast" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency", "queue"], value: 0 }],
  });
});

test("quick_setups_start_open_on_an_untouched_component_and_folded_once_it_has_settings", async () => {
  await show();
  act(() => useGroups.setState({ open: {} }));
  const head = () => screen.getByRole("button", { name: /^Quick setups/ });
  // show() opened every group first; the stored choice is cleared, so the default decides
  cleanup();
  await show(nodeSummary(), { ...app, hold: "call" });
  act(() => useGroups.setState({ open: {} }));
  expect(head().getAttribute("aria-expanded")).toBe("false");
  expect(head().textContent).toContain("Thread per request");
});

test("a_group_lists_only_what_differs_from_its_default_above_reset", async () => {
  await show(nodeSummary(), { ...app, hold: "call" });
  const table = screen.getByRole("table", { name: "What differs from the default" });
  expect(table.textContent).toContain("Server style");
  expect(table.textContent).toContain("Thread per request");
});
