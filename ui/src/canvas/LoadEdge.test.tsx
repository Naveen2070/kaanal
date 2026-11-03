import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { EdgeProps } from "@xyflow/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../data/api/types";
import { useDraft } from "../data/store/draft";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { FeedContext, type StatsFeed } from "./feed";
import { LoadEdge } from "./LoadEdge";

vi.mock("@xyflow/react", () => ({
  BaseEdge: (p: { path: string; style?: object }) => (
    <path data-testid="wire" d={p.path} style={p.style} />
  ),
  useNodes: () => [],
  EdgeLabelRenderer: ({ children }: { children: unknown }) => <>{children}</>,
  getSmoothStepPath: (o: { borderRadius: number; centerX?: number }) => [
    `M0 0 r${o.borderRadius} c${o.centerX ?? "mid"}`,
    5,
    5,
  ],
}));

const rep = (replica: number, visits_per_s: number) => ({
  replica,
  visits: 0,
  visits_per_s,
  mean_wait_ms: 0,
  offered_load: null,
  utilization: null,
});

function view(copies: number): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service", replicas: copies } },
      edges: [{ from: "gw", to: "app" }],
    },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

beforeEach(() => useDraft.setState({ view: view(1), error: null }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const feed = (busyTo: number, replicas = [rep(0, 100)], visits = 100): StatsFeed => ({
  node: (name) =>
    name === "gw"
      ? nodeSummary({ name: "gw", utilization: 0.1, visits_per_s: 100 })
      : name === "app"
        ? nodeSummary({ name: "app", utilization: busyTo, visits_per_s: visits, replicas })
        : undefined,
  nodes: [],
  dimmed: false,
  peakVisits: 100,
});

function edge(f: StatsFeed, selected = false) {
  const props = {
    id: "e0",
    sourceX: 0,
    sourceY: 0,
    targetX: 100,
    targetY: 10,
    sourcePosition: "right",
    targetPosition: "left",
    selected,
    data: { from: "gw", to: "app", index: 0 },
  };
  return render(
    <svg>
      <FeedContext.Provider value={f}>
        <LoadEdge {...(props as unknown as EdgeProps)} />
      </FeedContext.Provider>
    </svg>,
  );
}

/** The moving part of each lane (the one that carries the arrowhead). */
const lanes = () => [...document.querySelectorAll("path[marker-end]")] as SVGPathElement[];

test("a_single_copy_gets_one_lane_with_an_arrowhead_toward_the_target", () => {
  edge(feed(0.2));
  expect(lanes()).toHaveLength(1);
  expect(lanes()[0]?.getAttribute("marker-end")).toBe("url(#e0-arrow-0)");
  expect(document.querySelector("marker#e0-arrow-0")?.getAttribute("orient")).toBe("auto");
});

test("each_copy_of_the_target_is_its_own_lane_in_its_own_colour", () => {
  useDraft.setState({ view: view(3) });
  edge(feed(0.2, [rep(0, 10), rep(1, 50), rep(2, 100)]));
  expect(lanes().map((l) => l.getAttribute("stroke"))).toEqual([
    "var(--lane-0)",
    "var(--lane-1)",
    "var(--lane-2)",
  ]);
  // lanes bend at their own x, so they run side by side and never over each other
  const bends = [...document.querySelectorAll("path")]
    .map((p) => p.getAttribute("d") ?? "")
    .filter((d) => d.includes("c") && /c-?\d/.test(d));
  expect(new Set(bends).size).toBe(3);
});

test("a_lane_pulses_faster_and_heavier_the_more_its_copy_takes", () => {
  useDraft.setState({ view: view(2) });
  edge(feed(0.2, [rep(0, 10), rep(1, 100)]));
  const [slow, fast] = lanes();
  expect(slow?.getAttribute("class")).toMatch(/kaanal-lane/);
  expect(Number.parseFloat(slow?.style.animationDuration ?? "0")).toBeGreaterThan(
    Number.parseFloat(fast?.style.animationDuration ?? "0"),
  );
  expect(Number(fast?.getAttribute("stroke-width"))).toBeGreaterThan(
    Number(slow?.getAttribute("stroke-width")),
  );
});

test("a_lane_with_no_traffic_does_not_pulse", () => {
  edge(feed(0.1, [rep(0, 0)], 0));
  expect(lanes()[0]?.getAttribute("class") ?? "").not.toMatch(/kaanal-lane/);
});

test("lanes_stand_still_under_reduced_motion_and_the_state_stays_in_the_halo", () => {
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: q.includes("reduce"),
    addEventListener() {},
    removeEventListener() {},
  }));
  edge(feed(0.95));
  expect(lanes()[0]?.getAttribute("class") ?? "").not.toMatch(/kaanal-lane/);
  expect(document.querySelector('path[stroke="var(--bad-fill)"]')).not.toBeNull();
});

test("a_hot_target_puts_a_halo_under_the_wire_and_a_calm_one_does_not", () => {
  edge(feed(0.8));
  expect(document.querySelector('path[stroke="var(--warn-fill)"]')).not.toBeNull();
  cleanup();
  edge(feed(0.2));
  expect(
    document.querySelector('path[stroke="var(--warn-fill)"], path[stroke="var(--bad-fill)"]'),
  ).toBeNull();
});

test("a_full_wire_is_red_even_when_both_ends_are_idle", () => {
  const link = (saturated: boolean) => ({
    from: "gw",
    to: "app",
    transfers: 10,
    mean_wait_ms: 5,
    offered_load: saturated ? 3 : 0.1,
    utilization: saturated ? 1 : 0.1,
    saturated,
    saturates_at_req_s: 40,
    series: { window_s: 1, latency: [] },
  });
  edge({ ...feed(0.0), link: () => link(true) });
  expect(document.querySelector("path[data-wire-full]")?.getAttribute("stroke")).toBe(
    "var(--bad-fill)",
  );
  cleanup();
  edge({ ...feed(0.0), link: () => link(false) });
  expect(document.querySelector("path[data-wire-full]")).toBeNull();
  expect(document.querySelector('path[stroke="var(--bad-fill)"]')).toBeNull();
});

test("a_selected_wire_shows_a_button_that_deletes_exactly_that_entry", async () => {
  edge(feed(0.2));
  expect(screen.queryByRole("button", { name: /Delete the wire/ })).toBeNull();
  cleanup();
  mockFetch([{ json: view(1) }]);
  await useDraft.getState().load(); // the store must hold a confirmed draft before it takes an edit
  edge(feed(0.2), true);
  const sent = mockFetch([{ json: view(1) }]);
  fireEvent.click(screen.getByRole("button", { name: "Delete the wire from gw to app" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0] }] });
});
