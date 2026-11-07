import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import { BusyChart } from "./BusyChart";

afterEach(cleanup);

test("a_component_with_no_busy_series_draws_nothing", () => {
  const { container } = render(<BusyChart name="db" busy={[]} windowS={0.5} />);
  expect(container.firstChild).toBeNull();
});

test("the_sentence_gives_the_peak_and_the_last_window", () => {
  render(<BusyChart name="db" busy={[0.2, 0.95, 0.5]} windowS={0.5} />);
  expect(
    screen.getByTitle(/db busy over the run: peak 95% of its slots, last window 50%/),
  ).toBeTruthy();
});

test("the_time_axis_runs_for_every_window", () => {
  render(<BusyChart name="db" busy={Array(60).fill(0.5)} windowS={0.5} />);
  expect(screen.getByText("30 s")).toBeTruthy();
});
