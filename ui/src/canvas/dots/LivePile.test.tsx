import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { Tick } from "../../data/api/liveTypes";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { expectNoViolations } from "../../test/axe";
import { useDots } from "./dotStore";
import { LivePile } from "./LivePile";

const none = { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0 };
const tick = (queue: number, dropped = 0): Tick => ({
  t_ms: 250,
  system_ms: 250,
  requests: 1,
  failed: 0,
  nodes: [
    { arrived: 1, dropped: { ...none, overflow: dropped }, queue, busy_slots: 0, cpu_load: null },
  ],
  wires: [],
  hist: [],
  journeys: [],
  status: { rate: 1, paused: false, overloaded: false, in_flight: 0 },
});

beforeEach(() => {
  useDraft.setState({
    view: { doc: { nodes: { app: { type: "service" } }, edges: [] } } as unknown as DraftView,
  });
  useLive.getState().reset();
  useDots.getState().clear();
});
afterEach(cleanup);

test("nothing_waiting_and_nothing_dropped_draws_nothing", () => {
  useLive.setState({ ticks: [tick(0)] });
  const { container } = render(<LivePile name="app" />);
  expect(container.firstChild).toBeNull();
});

test("a_queue_is_a_pile_of_dots_and_words", async () => {
  useLive.setState({ ticks: [tick(30)] });
  const { container } = render(<LivePile name="app" />);
  expect(screen.getByText("30 waiting")).toBeTruthy();
  expect(container.querySelectorAll("[data-pile] > span")).toHaveLength(12); // the pile is capped
  await expectNoViolations(container);
});

test("a_drop_flashes_a_cross_and_says_how_many_were_turned_away", async () => {
  useLive.setState({ ticks: [tick(0, 4)] });
  const { container } = render(<LivePile name="app" />);
  const flash = container.querySelector("[data-flash]");
  expect(flash?.textContent).toContain("4 turned away");
  expect(flash?.textContent).toContain("×");
  await expectNoViolations(container);
});

test("a_component_the_run_does_not_have_draws_nothing", () => {
  useLive.setState({ ticks: [tick(5)] });
  useDraft.setState({
    view: { doc: { nodes: { other: { type: "service" } }, edges: [] } } as unknown as DraftView,
  });
  const { container } = render(<LivePile name="missing" />);
  expect(container.firstChild).toBeNull();
});

test("every_colour_in_the_live_canvas_files_is_a_token_or_a_utility_not_a_loose_hex", () => {
  const dir = join(process.cwd(), "src/canvas/dots");
  const files = readdirSync(dir).filter((f) => /\.(ts|tsx)$/.test(f) && !f.includes(".test."));
  expect(files.length).toBeGreaterThan(3);
  for (const f of files) {
    const text = readFileSync(join(dir, f), "utf8");
    expect(text.match(/#[0-9a-fA-F]{3,8}/g) ?? [], f).toEqual([]);
  }
  const css = readFileSync(join(process.cwd(), "src/main.css"), "utf8");
  const from = css.indexOf("@keyframes kaanal-dot");
  const rules = css.slice(from, css.indexOf("@media (prefers-reduced-motion: reduce)", from));
  expect(rules.length).toBeGreaterThan(200);
  expect(rules.match(/#[0-9a-fA-F]{3,8}/g) ?? []).toEqual([]);
});

test("the_pile_follows_the_streams_component_order_not_the_drafts", () => {
  // the draft lists app before gw (alphabetical); the stream says gw, then app
  useDraft.setState({
    view: {
      doc: { nodes: { app: { type: "service" }, gw: { type: "gateway" } }, edges: [] },
    } as unknown as DraftView,
  });
  const two: Tick = {
    ...tick(0),
    nodes: [
      { arrived: 1, dropped: none, queue: 0, busy_slots: 0, cpu_load: null },
      { arrived: 1, dropped: none, queue: 9, busy_slots: 0, cpu_load: null },
    ],
  };
  useLive.setState({
    ticks: [two],
    hello: {
      session: 1,
      profile: null,
      note: null,
      rate: 1,
      paused: false,
      overloaded: false,
      in_flight: 0,
      submitted: 0,
      clients: 1,
      system_ms: 0,
      nodes: ["gw", "app"],
    },
  });
  render(<LivePile name="app" />);
  expect(screen.getByText("9 waiting")).toBeTruthy();
  cleanup();
  const { container } = render(<LivePile name="gw" />);
  expect(container.firstChild).toBeNull();
});
