import { cleanup, render, screen } from "@testing-library/react";
import type { NodeProps } from "@xyflow/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { expectNoViolations } from "../test/axe";
import { nodeSummary } from "../test/summary";
import { FeedContext, type StatsFeed } from "./feed";
import { NodeCard } from "./NodeCard";

vi.mock("@xyflow/react", () => ({
  Handle: () => null,
  Position: { Left: "left", Right: "right" },
}));

const config = { type: "service", concurrency: 12, proc: "98ms" };

function card(feed: StatsFeed, cfg: Record<string, unknown> = config) {
  const props = { data: { name: "app", kind: cfg.type, config: cfg }, selected: false };
  render(
    <FeedContext.Provider value={feed}>
      <NodeCard {...(props as unknown as NodeProps)} />
    </FeedContext.Provider>,
  );
}

const feedOf = (n: ReturnType<typeof nodeSummary> | undefined, dimmed = false): StatsFeed => ({
  node: (name) => (name === "app" ? n : undefined),
  dimmed,
  peakVisits: n?.visits_per_s ?? 0,
});

afterEach(cleanup);

test("a_card_with_no_run_says_no_run_yet", () => {
  card(feedOf(undefined));
  expect(screen.getByText("No run yet")).toBeTruthy();
  expect(screen.queryByRole("meter")).toBeNull();
  expect(screen.getByText("Arriving").nextElementSibling?.textContent).toBe("–");
  expect(screen.getByText("Slowest work under").nextElementSibling?.textContent).toBe("–");
});

test("card_shows_config_line_and_up_to_n_waiting", () => {
  card(feedOf(nodeSummary({ utilization: 0.78, max_queue: 12, visits_per_s: 1000, p99_ms: 120 })));
  expect(screen.getByText("Service · 1 copy × 12 slots · 98 ms")).toBeTruthy();
  expect(screen.getByText("78% busy · up to 12 waiting")).toBeTruthy();
  expect(screen.getByText("Arriving").nextElementSibling?.textContent).toBe("1,000/s");
  expect(screen.getByText("Slowest work under").nextElementSibling?.textContent).toBe("120 ms");
  expect(screen.getByRole("meter").getAttribute("value")).toBe("78");
});

test("card_group_label_includes_state", () => {
  card(feedOf(nodeSummary({ utilization: 0.78 })));
  expect(screen.getByRole("group", { name: "app, Service, 78 percent busy" })).toBeTruthy();
});

test("the_group_label_says_saturated_idle_and_no_run_in_words", () => {
  for (const [feed, words] of [
    [feedOf(nodeSummary({ saturated: true, utilization: 1 })), "saturated"],
    [feedOf(nodeSummary({ utilization: 0, visits_per_s: 0 })), "idle"],
    [feedOf(undefined), "no run yet"],
  ] as const) {
    card(feed);
    expect(screen.getByRole("group").getAttribute("aria-label")).toBe(`app, Service, ${words}`);
    cleanup();
  }
});

test("card_without_slots_says_not_set_never_saturates", () => {
  card(feedOf(nodeSummary({ utilization: null, saturates_at_req_s: null })), { type: "gateway" });
  expect(screen.getByText("No slot limit · never saturates")).toBeTruthy();
  expect(screen.getByText("Gateway · 1 copy · no slot limit")).toBeTruthy();
});

test("the_header_is_a_button_for_selection_and_the_card_itself_is_not", () => {
  card(feedOf(undefined));
  const header = screen.getByRole("button", { name: /app/ });
  expect(header.closest("fieldset")).not.toBeNull();
  expect(screen.getByRole("group").tagName).toBe("FIELDSET");
});

test("a_saturated_node_is_red_and_says_why_in_text_not_only_colour", () => {
  card(feedOf(nodeSummary({ saturated: true, utilization: 1, max_queue: 40 })));
  expect(screen.getByText("Saturated · up to 40 waiting")).toBeTruthy();
  expect(document.querySelector(".border-negative")).not.toBeNull();
});

test("copies_show_in_the_config_line_and_a_stale_feed_marks_the_figures", () => {
  card(feedOf(nodeSummary(), true), { ...config, replicas: 3 });
  expect(screen.getByText("Service · 3 copies × 12 slots · 98 ms")).toBeTruthy();
  expect(screen.getByText("updating")).toBeTruthy(); // stale_marker_shows_while_a_run_is_pending
  expect(document.querySelector(".opacity-60")).not.toBeNull();
});

test("the_card_has_no_axe_violations_idle_busy_or_saturated", async () => {
  for (const n of [
    undefined,
    nodeSummary({ utilization: 0.5 }),
    nodeSummary({ saturated: true, utilization: 1 }),
  ]) {
    card(feedOf(n));
    await expectNoViolations(document.body);
    cleanup();
  }
});

test("a_distribution_and_a_database_have_their_own_config_words", () => {
  card(feedOf(undefined), { type: "service", concurrency: 4, proc: { p50: "4ms", p99: "20ms" } });
  expect(screen.getByText("Service · 1 copy × 4 slots · time varies")).toBeTruthy();
  cleanup();
  card(feedOf(undefined), { type: "database", pool: { size: 10 } });
  expect(screen.getByText("Database · 1 copy × 10 slots")).toBeTruthy(); // no Time row for a database
});

// the card reads a feed; if it imported the run store a later live stream would have to rewrite it
const sources = import.meta.glob("./*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

test("the_card_does_not_import_the_run_store", () => {
  for (const f of ["./NodeCard.tsx", "./ControlBar.tsx", "./Meter.tsx", "./metrics.ts"]) {
    expect(sources[f], f).toBeDefined();
    expect(sources[f], f).not.toMatch(/store\/run|useRun/);
  }
});
