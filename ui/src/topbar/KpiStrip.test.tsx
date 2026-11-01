import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, Summary } from "../api/types";
import { rpsFromSlider, sliderFromRps } from "../canvas/logScale";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { expectNoViolations } from "../test/axe";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { KpiStrip } from "./KpiStrip";

function view(rps: number | null): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {}, ...(rps === null ? {} : { traffic: { peak: { rps, journeys: [] } } }) },
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

function summary(over: Partial<Summary> = {}): Summary {
  return {
    label: "relative what-if",
    window_s: 10,
    requests: 1000,
    failed: 100,
    errors: {},
    warnings: [],
    latency: { p50_ms: 1, p95_ms: 5, p99_ms: 12.34, max_ms: 20, buckets: [] },
    series: { window_s: 1, done: [], failed: [], p99_ms: [] },
    streams: [],
    nodes: [nodeSummary({ name: "db", limited: 3, overflowed: 4, saturates_at_req_s: 1280 })],
    links: [],
    bottlenecks: [0],
    ...over,
  } as Summary;
}

function run(s: Summary | null, dimmed = false) {
  act(() => runState(s, dimmed));
}

function runState(s: Summary | null, dimmed: boolean) {
  useRun.setState({
    status: "done",
    dimmed,
    lastError: null,
    latest: s
      ? {
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
        }
      : null,
  });
}

async function show(rps: number | null) {
  mockFetch([{ json: view(rps) }]);
  await useDraft.getState().load();
  render(<KpiStrip />);
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const value = (label: string) => screen.getByText(label).nextElementSibling as HTMLElement;
const slider = () => screen.getByRole("slider", { name: "Offered load" }) as HTMLInputElement;
const typedBox = () => screen.getByLabelText(/typed/i) as HTMLInputElement;

test("the_cells_come_in_the_order_breaks_first_load_p99_goodput_errors_dropped", async () => {
  await show(300);
  const labels = [...document.querySelectorAll("dt")].map((d) => d.textContent);
  expect(labels).toEqual([
    "Breaks first",
    "Offered load",
    "System p99",
    "Goodput",
    "Errors",
    "Dropped",
  ]);
});

test("the_figures_come_from_the_newest_run", async () => {
  await show(300);
  run(summary());
  expect(await screen.findByText("12.3 ms")).toBeTruthy();
  expect(value("Goodput").textContent).toBe("90/s"); // (1000 - 100) / 10 s
  expect(value("Errors").textContent).toBe("100");
  expect(value("Dropped").textContent).toBe("7"); // 3 + 4
  expect(value("Breaks first").textContent).toBe("db at 1,300 req/s"); // two significant digits
});

test("breaks_first_is_the_largest_figure_and_red_only_when_saturated_now", async () => {
  await show(300);
  run(summary());
  const lead = value("Breaks first");
  expect(lead.className).toMatch(/text-xl/);
  for (const other of ["System p99", "Goodput", "Errors", "Dropped"]) {
    expect(value(other).className).not.toMatch(/text-xl/);
  }
  expect(lead.className).not.toMatch(/text-negative/);
  run(
    summary({
      nodes: [
        nodeSummary({ name: "db", saturated: true, saturates_at_req_s: 120, utilization: 1 }),
      ],
    }),
  );
  expect(value("Breaks first").className).toMatch(/text-negative/);
  expect(value("Breaks first").textContent).toContain("db at 120 req/s");
});

test("nothing_saturates_is_grey_text_not_a_number", async () => {
  await show(300);
  run(summary({ nodes: [nodeSummary({ saturates_at_req_s: null })] }));
  expect(value("Breaks first").textContent).toBe("nothing saturates");
  expect(value("Breaks first").className).toMatch(/text-muted/);
});

test("before_any_run_the_lead_says_to_run_not_that_nothing_saturates", async () => {
  await show(300);
  expect(value("Breaks first").textContent).toBe("Run to find out");
});

test("null_p99_prints_a_grey_dash_not_zero", async () => {
  await show(300);
  expect(value("System p99").textContent).toBe("–"); // no run yet
  run(summary({ requests: 0, failed: 0 }));
  expect(value("System p99").textContent).toBe("–"); // a run with no requests
  expect(value("System p99").className).toMatch(/text-muted/);
  expect(value("Goodput").textContent).toBe("0/s");
  expect(document.body.textContent).not.toMatch(/NaN|Infinity/);
});

test("a_stale_run_dims_the_figures", async () => {
  await show(300);
  run(summary(), true);
  expect(document.querySelector(".opacity-60")).not.toBeNull();
});

test("a_draft_with_no_profile_has_a_disabled_slider_and_dashes", async () => {
  await show(null);
  expect(slider().disabled).toBe(true);
  expect(typedBox().disabled).toBe(true);
});

test("slider_commits_on_release_only", async () => {
  await show(300);
  const sent = mockFetch([{ json: view(2000) }]);
  const target = sliderFromRps(2000);
  fireEvent.change(slider(), { target: { value: String(target) } });
  expect(sent).toHaveLength(0); // dragging sends nothing
  expect(typedBox().value).toBe(String(rpsFromSlider(target))); // the figure follows the thumb
  fireEvent.pointerUp(slider());
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "rps"], value: rpsFromSlider(target) }],
  });
});

test("an_arrow_key_commits_and_the_slider_keeps_focus", async () => {
  await show(300);
  slider().focus();
  const sent = mockFetch([{ json: view(301) }]);
  fireEvent.change(slider(), { target: { value: String(sliderFromRps(300) + 12) } });
  fireEvent.keyUp(slider(), { key: "ArrowRight" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(document.activeElement).toBe(slider());
});

test("tabbing_onto_the_slider_patches_nothing", async () => {
  await show(300);
  const sent = mockFetch([{ json: view(300) }]);
  fireEvent.keyUp(slider(), { key: "Tab" });
  fireEvent.pointerUp(slider());
  expect(sent).toHaveLength(0); // the value did not change, so there is nothing to write
});

test("typed_value_is_clamped_to_R1_100_and_shown_exactly", async () => {
  await show(300);
  const sent = mockFetch([{ json: view(1_000_000) }]);
  fireEvent.change(typedBox(), { target: { value: "2500000" } });
  fireEvent.keyDown(typedBox(), { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "rps"], value: 1_000_000 }],
  });
  // a typed 12.6 is a whole number: the figure shown is the figure sent
  const sent2 = mockFetch([{ json: view(13) }]);
  fireEvent.change(typedBox(), { target: { value: "12.6" } });
  fireEvent.blur(typedBox());
  await vi.waitFor(() => expect(sent2).toHaveLength(1));
  expect(sent2[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "rps"], value: 13 }],
  });
});

test("typing_nothing_or_nonsense_sends_nothing_and_puts_the_real_value_back", async () => {
  await show(300);
  const sent = mockFetch([{ json: view(300) }]);
  for (const bad of ["", "abc", "-4", "0"]) {
    fireEvent.change(typedBox(), { target: { value: bad } });
    fireEvent.blur(typedBox());
  }
  expect(sent).toHaveLength(0);
  expect(typedBox().value).toBe("300");
});

test("the_breaks_first_marker_sits_on_the_slider_at_the_log_position", async () => {
  await show(300);
  run(summary());
  const m = screen.getByTitle("Breaks first at 1,300 req/s");
  expect(m.style.left).toBe(`${(sliderFromRps(1280) / 1000) * 100}%`);
});

test("the_strip_has_no_axe_violations", async () => {
  await show(300);
  run(summary());
  await expectNoViolations(document.body);
});
