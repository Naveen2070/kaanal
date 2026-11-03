import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { RunRecord, Series, Summary } from "../../data/api/types";
import { useRun } from "../../data/store/run";
import { nodeSummary } from "../../test/summary";
import { bucketLabel, heatmapData, intensity } from "./heatmapData";
import { Results } from "./Results";
import { HEAT_ASPECT, RANK_ROW } from "./Skeletons";

const buckets = Array.from({ length: 20 }, (_, i) => ({
  le_ms: i < 19 ? 2 ** i / 8 : null,
  count: 0,
}));

function series(cells: [number, number, number][] = []): Series {
  // 60 windows x 20 buckets, all zero except the given (window, bucket, count)
  const latency = Array.from({ length: 60 }, () => Array<number>(20).fill(0));
  for (const [w, b, n] of cells) (latency[w] as number[])[b] = n;
  return { window_s: 0.5, busy: [], queue: [], latency };
}

function summary(over: Partial<Summary> = {}): Summary {
  return {
    label: "relative what-if, not a prediction",
    window_s: 30,
    requests: 100,
    failed: 5,
    errors: { overflow: 3, timeout: 2 },
    warnings: [],
    latency: { p50_ms: 1, p95_ms: 2, p99_ms: 3, max_ms: 4, buckets },
    series: { window_s: 1, done: [], failed: [], p99_ms: [] },
    streams: [],
    nodes: [
      nodeSummary({ name: "gw", series: series() }),
      nodeSummary({
        name: "db",
        offered_load: 1.4,
        series: series([
          [3, 5, 40],
          [4, 9, 2],
        ]),
      }),
    ],
    links: [],
    bottlenecks: [1, 0],
    ...over,
  };
}

function setRun(s: Summary | undefined) {
  const r: RunRecord = {
    id: "s1",
    state: "done",
    generation: 1,
    profile: "peak",
    scenario: null,
    source: "draft",
    draft_rev: 0,
    started_ms: 0,
    finished_ms: 1,
    summary: s,
  };
  useRun.setState({ status: "done", latest: r, dimmed: false, lastError: null });
}

function mount() {
  history.replaceState(null, "", "/__kaanal/studio");
  render(
    <NuqsAdapter>
      <Results />
    </NuqsAdapter>,
  );
}

beforeEach(() => useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null }));
afterEach(cleanup);

test("heatmap_data_has_one_row_per_bucket_and_sixty_columns", () => {
  const d = heatmapData(series([[3, 5, 40]]));
  expect(d.cols).toBe(60);
  expect(d.rows).toBe(20);
  expect(d.cells).toHaveLength(20);
  expect(d.cells.every((row) => row.length === 60)).toBe(true);
  expect(d.cells[5]?.[3]).toBe(40);
  expect(d.max).toBe(40);
});

test("the_scale_is_logarithmic_and_bounded", () => {
  expect(intensity(0, 100)).toBe(0);
  expect(intensity(100, 100)).toBe(1);
  // a hop in a million still shows next to the busiest cell
  expect(intensity(1, 1_000_000)).toBeGreaterThan(0.04);
  expect(intensity(10, 1_000_000)).toBeLessThan(intensity(1000, 1_000_000));
  expect(intensity(5, 0)).toBe(0);
});

test("a_ragged_or_empty_series_does_not_break_the_data", () => {
  expect(heatmapData({ window_s: 1, busy: [], queue: [], latency: [] })).toEqual({
    cols: 0,
    rows: 0,
    cells: [],
    max: 0,
  });
  const ragged = heatmapData({ window_s: 1, busy: [], queue: [], latency: [[1], [1, 2, 3]] });
  expect(ragged.rows).toBe(3);
  expect(ragged.cells[2]).toEqual([0, 3]);
});

test("buckets_are_labelled_by_their_upper_bound", () => {
  expect(bucketLabel(buckets, 3)).toBe("<= 1 ms");
  expect(bucketLabel(buckets, 19)).toBe("slower");
});

test("an_all_zero_series_renders_an_empty_state_not_a_blank", () => {
  setRun(summary());
  mount();
  // the first bottleneck is db (has hops), so pick gw, which has none
  const select = screen.getByLabelText(/Component/) as HTMLSelectElement;
  select.value = "gw";
  select.dispatchEvent(new Event("change", { bubbles: true }));
  return screen.findByText(/No requests reached gw/);
});

test("the_heatmap_draws_the_busiest_component_first_with_a_table_fallback", () => {
  setRun(summary());
  mount();
  expect((screen.getByLabelText(/Component/) as HTMLSelectElement).value).toBe("db");
  expect(document.querySelectorAll("svg rect")).toHaveLength(2);
  expect(screen.getByRole("table")).toBeTruthy();
  expect(screen.getByText("Hops of db by latency")).toBeTruthy();
});

test("the_warning_text_is_visible_above_the_heatmap", () => {
  setRun(summary({ warnings: ["fewer than 1000 samples: p99 is noise"] }));
  mount();
  const warning = screen.getByText("fewer than 1000 samples: p99 is noise");
  const svg = document.querySelector("svg") as SVGElement;
  expect(warning.compareDocumentPosition(svg) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("breaks_first_list_ranks_by_peak_with_saturates_at", () => {
  setRun(
    summary({
      nodes: [
        nodeSummary({ name: "gw", series: series(), saturates_at_req_s: null }),
        nodeSummary({
          name: "db",
          saturates_at_req_s: 1280,
          series: { ...series(), busy: [0.4, 0.95] },
        }),
      ],
    }),
  );
  mount();
  const items = Array.from(document.querySelectorAll("section ol li")).map((l) => l.textContent);
  expect(items[0]).toBe("dbpeak 95%saturates at 1,300 req/s");
  expect(items[1]).toMatch(/^gw/);
  expect(items[1]).not.toMatch(/saturates at/);
  expect(items[1]).toMatch(/no saturation point/);
});

const wireOf = (over = {}) => ({
  from: "gw",
  to: "db",
  transfers: 10,
  mean_wait_ms: 5,
  offered_load: 3,
  utilization: 1,
  saturated: true,
  saturates_at_req_s: 1.1,
  series: { window_s: 0.5, latency: series([[2, 7, 9]]).latency },
  ...over,
});

test("a_full_wire_is_ranked_with_the_components_and_marked_as_a_wire", () => {
  setRun(summary({ links: [wireOf()] }));
  mount();
  const items = Array.from(document.querySelectorAll("section ol li")).map((l) => l.textContent);
  // the wire peaks at 100%, above db's, so it comes first
  expect(items[0]).toBe("gw → dbpeak 100%Wire · saturates at 1.1 req/s");
});

test("the_heatmap_can_show_one_wire_and_says_what_its_time_is", () => {
  setRun(summary({ links: [wireOf()] }));
  mount();
  const select = screen.getByLabelText(/Component or wire/) as HTMLSelectElement;
  fireEvent.change(select, {
    target: { value: select.querySelector("optgroup[label=Wires] option")?.getAttribute("value") },
  });
  expect(screen.getByText("Bodies on gw → db by time on the wire")).toBeTruthy();
  expect(
    screen.getByText(/queue plus the transfer|wait in the queue plus the transfer/),
  ).toBeTruthy();
  expect(document.querySelectorAll("svg rect")).toHaveLength(1);
});

test("a_wire_with_nothing_across_it_says_so_not_a_blank_heatmap", () => {
  const quiet = wireOf({ series: { window_s: 1, latency: [[0, 0]] } });
  setRun(summary({ links: [quiet] }));
  mount();
  const select = screen.getByLabelText(/Component or wire/) as HTMLSelectElement;
  fireEvent.change(select, {
    target: { value: select.querySelector("optgroup[label=Wires] option")?.getAttribute("value") },
  });
  expect(screen.getByText(/Nothing crossed gw → db/)).toBeTruthy();
});

test("first_row_red_others_ink", () => {
  setRun(summary());
  mount();
  const fills = Array.from(document.querySelectorAll("section ol li div[aria-hidden] > div"));
  expect(fills[0]?.className).toMatch(/bg-bad-fill/);
  expect(fills[1]?.className).toMatch(/bg-text/);
});

test("by_instance_shows_only_visits_utilisation_wait_and_hides_for_one_copy", () => {
  const rep = {
    replica: 0,
    visits: 5,
    visits_per_s: 5,
    mean_wait_ms: 1,
    offered_load: 0.5,
    utilization: 0.5,
  };
  setRun(
    summary({
      nodes: [nodeSummary({ name: "db", replicas: [rep, { ...rep, replica: 1 }] })],
      bottlenecks: [0],
    }),
  );
  mount();
  const heads = Array.from(document.querySelectorAll("table:not(.sr-only) th")).map(
    (h) => h.textContent,
  );
  expect(heads).toEqual(["Instance", "Visits/s", "Busy", "Wait"]);
  cleanup();
  setRun(summary({ nodes: [nodeSummary({ name: "db", replicas: [rep] })], bottlenecks: [0] }));
  mount();
  expect(screen.queryByText(/Instances of/)).toBeNull();
});

test("zero_errors_print_a_zero", () => {
  setRun(summary({ errors: {} }));
  mount();
  const section = screen.getByText("Errors by kind").closest("section") as HTMLElement;
  expect(section.textContent).toContain("0");
  expect(section.textContent).not.toMatch(/none|no errors/i);
});

test("heatmap_axis_has_start_middle_and_end", () => {
  setRun(summary());
  mount();
  expect(screen.getByText("0 s")).toBeTruthy();
  expect(screen.getByText("15 s")).toBeTruthy(); // 60 windows of 0.5 s
  expect(screen.getByText("30 s")).toBeTruthy();
});

test("loading_skeleton_has_the_final_layout_size", () => {
  useRun.setState({ status: "running", latest: null });
  mount();
  const heat = screen
    .getByLabelText("Loading the results")
    .querySelector(`[class*="${HEAT_ASPECT}"]`);
  expect(heat).toBeTruthy();
  cleanup();
  setRun(summary());
  mount();
  expect(document.querySelector("svg")?.getAttribute("class")).toContain(HEAT_ASPECT);
  expect(document.querySelector("section ol li")?.className).toContain(RANK_ROW);
});

test("the_report_label_is_always_present", () => {
  setRun(summary({ nodes: [], bottlenecks: [] }));
  mount();
  expect(screen.getByText(/Run 1 · A what-if, not a prediction/)).toBeTruthy();
  expect(screen.getByText(/This run has no components/)).toBeTruthy();
});

test("errors_by_kind_and_no_run_are_both_said_plainly", () => {
  setRun(summary());
  mount();
  expect(screen.getByText("overflow: 3")).toBeTruthy();
  cleanup();
  setRun(undefined);
  mount();
  expect(screen.getByText(/No finished run yet/)).toBeTruthy();
});

test("a_pick_in_the_dropdown_gives_way_when_the_canvas_selection_changes", () => {
  setRun(summary());
  history.replaceState(null, "", "/__kaanal/studio");
  render(
    <NuqsAdapter>
      <Results />
    </NuqsAdapter>,
  );
  const select = screen.getByLabelText(/Component/) as HTMLSelectElement;
  expect(select.value).toBe("db"); // the first bottleneck
  fireEvent.change(select, { target: { value: "gw" } });
  expect(select.value).toBe("gw");
  // the user selects db on the canvas: the heatmap follows, the earlier pick no longer wins
  act(() => {
    history.replaceState(null, "", "/__kaanal/studio?node=db");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  expect(select.value).toBe("db");
});
