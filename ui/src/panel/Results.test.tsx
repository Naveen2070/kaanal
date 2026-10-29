import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { RunRecord, Series, Summary } from "../api/types";
import { useRun } from "../store/run";
import { nodeSummary } from "../test/summary";
import { bucketLabel, heatmapData, intensity } from "./heatmapData";
import { Results } from "./Results";

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

test("bottlenecks_are_listed_in_server_order", () => {
  setRun(summary());
  mount();
  const items = Array.from(document.querySelectorAll("ol.list-decimal li")).map(
    (l) => l.textContent,
  );
  expect(items).toEqual(["db (140% offered load)", "gw (50% offered load)"]);
});

test("the_report_label_is_always_present", () => {
  setRun(summary({ label: "relative what-if, not a prediction", nodes: [], bottlenecks: [] }));
  mount();
  expect(screen.getByText("relative what-if, not a prediction")).toBeTruthy();
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
