import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { Marker, Tick } from "../../data/api/liveTypes";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { MAX_WINDOW_MIN, MIN_WINDOW_MIN, useLive } from "../../data/store/live";
import { bucketOf } from "../../data/store/liveMath";
import { expectNoViolations } from "../../test/axe";
import { LiveResults } from "./LiveResults";
import { LiveTimeline } from "./LiveTimeline";
import { useGroups } from "./groups/store";
import { LiveWindow } from "./LiveWindow";

const none = { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0 };
const tick = (t_ms: number, busy: number[], queue = 0, failed = 0): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: 25,
  failed,
  nodes: busy.map((b) => ({ arrived: 25, dropped: none, queue, busy_slots: b, cpu_load: null })),
  wires: [],
  hist: [[bucketOf(20e6 + t_ms * 1000), 20]],
  journeys: [],
  status: { rate: 100, paused: false, overloaded: false, in_flight: 0 },
});
const doc = {
  nodes: {
    gw: { type: "gateway", concurrency: 10 },
    app: { type: "service", concurrency: 10 },
  },
  edges: [],
} as unknown as DraftView["doc"];

beforeEach(() => {
  useDraft.setState({ view: { doc } as unknown as DraftView });
  useLive.getState().reset();
  useLive.setState({ windowMin: MIN_WINDOW_MIN });
});
afterEach(cleanup);

test("live_results_rank_by_peak_over_the_window_and_leave_batch_widgets_out", async () => {
  // gw peaks at 30%, app at 90% earlier in the window though it is calm now
  const ticks = [tick(250, [3, 9]), tick(500, [3, 2]), tick(750, [3, 2])];
  useLive.setState({ ticks });
  const { container } = render(<LiveResults />, { wrapper: NuqsAdapter });
  const names = [...container.querySelectorAll("ol li span.font-medium")].map((e) => e.textContent);
  expect(names).toEqual(["app", "gw"]);
  expect(container.textContent).toContain("peak 90%");
  expect(container.textContent).not.toContain("Plan for a rate");
  expect(container.textContent).not.toContain("Share this run");
  await expectNoViolations(container);
});

test("a_note_from_the_server_is_shown_and_an_empty_run_says_so", () => {
  useLive.setState({
    hello: {
      session: 1,
      profile: null,
      note: "the file has no traffic profile",
      rate: 0,
      paused: false,
      overloaded: false,
      in_flight: 0,
      submitted: 0,
      clients: 1,
      system_ms: 0,
    },
  });
  render(<LiveResults />, { wrapper: NuqsAdapter });
  expect(screen.getByText("the file has no traffic profile")).toBeTruthy();
  expect(screen.getByText("No component has a figure yet.")).toBeTruthy();
});

test("the_timeline_draws_three_charts_and_names_every_change_in_words", () => {
  const ticks = Array.from({ length: 40 }, (_, i) =>
    tick((i + 1) * 250, [1, 1], 0, i > 20 ? 3 : 0),
  );
  const markers: Marker[] = [
    { at_ms: 3000, kind: "param", text: "app" },
    { at_ms: 6000, kind: "pause", text: "paused" },
    { at_ms: 9000, kind: "chaos", text: "slow db: activate" },
  ];
  useLive.setState({ ticks, markers });
  const { container } = render(<LiveTimeline />);
  expect(container.querySelectorAll("svg polyline")).toHaveLength(3);
  expect(container.querySelectorAll("svg line")).toHaveLength(9); // three marks on each chart
  const list = screen.getByRole("list", { name: "Changes on the timeline" }).textContent ?? "";
  expect(list).toContain("0:03 You changed: app");
  expect(list).toContain("0:06 Paused");
  expect(list).toContain("0:09 Chaos: slow db: activate");
});

test("a_run_that_has_just_started_says_the_timeline_fills_in", () => {
  useLive.setState({ ticks: [tick(250, [1, 1])] });
  render(<LiveTimeline />);
  expect(screen.getByText("The timeline fills in as the run goes on.")).toBeTruthy();
});

test("the_window_setting_is_5_to_30_minutes_and_resets_to_5", () => {
  useGroups.setState({ open: { "doc.live": true } });
  render(<LiveWindow />);
  const box = screen.getByLabelText("Keep the last (minutes)") as HTMLInputElement;
  expect(box.value).toBe("5");
  fireEvent.change(box, { target: { value: "20" } });
  expect(useLive.getState().windowMin).toBe(20);
  fireEvent.change(box, { target: { value: "500" } });
  expect(useLive.getState().windowMin).toBe(MAX_WINDOW_MIN);
  fireEvent.change(box, { target: { value: "1" } });
  expect(useLive.getState().windowMin).toBe(MIN_WINDOW_MIN);
});

test("the_window_group_waits_for_a_file", () => {
  useDraft.setState({ view: null });
  const { container } = render(<LiveWindow />);
  expect(container.firstChild).toBeNull();
});

// review finding 9: the restart marker sits at the start of the new run, before its first tick
test("the_restart_marker_at_the_start_of_a_run_is_listed", () => {
  const ticks = Array.from({ length: 10 }, (_, i) => tick((i + 1) * 250, [1, 1]));
  useLive.setState({
    ticks,
    markers: [{ at_ms: 0, kind: "structure", text: "Applied: the live run restarted" }],
  });
  render(<LiveTimeline />);
  expect(screen.getByRole("list", { name: "Changes on the timeline" }).textContent).toContain(
    "Applied: the live run restarted",
  );
});

test("errors_by_kind_add_the_window_and_measure_is_offered_without_a_run_yet", () => {
  const t = tick(250, [3, 3]);
  const node = t.nodes[0] as Tick["nodes"][number];
  useLive.setState({
    ticks: [
      {
        ...t,
        nodes: [{ ...node, dropped: { ...node.dropped, overflow: 7 } }, ...t.nodes.slice(1)],
      },
    ],
  });
  render(<LiveResults />, { wrapper: NuqsAdapter });
  expect(screen.getByText("queue full").nextSibling?.textContent).toBe("7");
  expect(screen.getByRole("button", { name: "Measure the draft" })).toBeTruthy();
});
