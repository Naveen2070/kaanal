import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { RunRecord } from "../../data/api/types";
import { useRun } from "../../data/store/run";
import { ChartsBand } from "./ChartsBand";

function setRun(done: number[] = [10, 20, 87], p99: number[] = [5, 48, 12]) {
  const r = {
    id: "s1",
    state: "done",
    summary: { series: { window_s: 0.5, done, failed: done.map(() => 0), p99_ms: p99 } },
  } as unknown as RunRecord;
  useRun.setState({ status: "done", latest: r, dimmed: false, lastError: null });
}

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("kaanal.studio.charts.open", "1"); // the band starts closed; most tests want it open
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(cleanup);

test("chart_has_a_y_scale_and_a_text_alternative", () => {
  setRun();
  render(<ChartsBand />);
  const charts = Array.from(document.querySelectorAll("svg > title"));
  expect(charts).toHaveLength(3);
  expect(charts[0]?.textContent).toBe("p99 latency: peak 48 ms, last window 12 ms, over 3 windows");
  expect(screen.getByText("100")).toBeTruthy(); // goodput's top tick above 87
  expect(document.querySelectorAll("polyline")).toHaveLength(3);
});

test("zero_run_draws_a_flat_line_with_no_nan", () => {
  setRun([0, 0], [0, 0]);
  render(<ChartsBand />);
  expect(document.body.innerHTML).not.toMatch(/NaN/);
});

test("before_any_run_it_says_so", () => {
  render(<ChartsBand />);
  expect(screen.getByText(/appear after the first run/)).toBeTruthy();
});

test("band_is_collapsible_and_remembers_state", () => {
  setRun();
  const { unmount } = render(<ChartsBand />);
  fireEvent.click(screen.getByRole("button", { name: "Hide charts" }));
  expect(document.querySelectorAll("figure")).toHaveLength(0);
  unmount();
  render(<ChartsBand />);
  const b = screen.getByRole("button", { name: "Show charts" });
  expect(b.getAttribute("aria-expanded")).toBe("false");
});

test("the_band_starts_closed_so_the_canvas_stays_clear", () => {
  localStorage.clear();
  setRun();
  render(<ChartsBand />);
  expect(screen.getByRole("button", { name: "Show charts" }).getAttribute("aria-expanded")).toBe(
    "false",
  );
  expect(document.querySelectorAll("figure")).toHaveLength(0);
});

test("the_stats_pill_sits_beside_the_toggle_open_or_closed", () => {
  render(<ChartsBand />);
  const pill = document.querySelector('[data-testid="system-stats"]');
  expect(pill).not.toBeNull();
  expect(pill?.textContent).toMatch(/components?/i);
  expect(pill?.textContent).toMatch(/connections?/i);
});

test("the_pill_says_how_long_the_last_re_run_took", () => {
  setRun();
  useRun.setState({
    status: "done",
    latest: { ...(useRun.getState().latest as RunRecord), started_ms: 10, finished_ms: 410 },
  });
  render(<ChartsBand />);
  expect(document.querySelector('[data-testid="system-stats"]')?.textContent).toMatch(
    /re-run in 0.4 s/,
  );
});
