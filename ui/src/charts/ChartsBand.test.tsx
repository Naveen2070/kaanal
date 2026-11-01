import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { RunRecord } from "../api/types";
import { useRun } from "../store/run";
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
  expect(document.querySelectorAll("svg")).toHaveLength(0);
  unmount();
  render(<ChartsBand />);
  const b = screen.getByRole("button", { name: "Show charts" });
  expect(b.getAttribute("aria-expanded")).toBe("false");
});
