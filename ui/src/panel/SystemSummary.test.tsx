import { cleanup, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, expect, test } from "vite-plus/test";
import { EMPTY_FEED, FeedContext, type StatsFeed } from "../canvas/feed";
import { nodeSummary } from "../test/summary";
import { SystemSummary } from "./SystemSummary";

afterEach(cleanup);

const withNodes = (nodes: ReturnType<typeof nodeSummary>[]): StatsFeed => ({
  ...EMPTY_FEED,
  nodes,
});
const busy = (name: string, peak: number, at: number | null) =>
  nodeSummary({
    name,
    utilization: peak,
    saturates_at_req_s: at,
    series: { ...nodeSummary().series, busy: [0, peak] },
  });

const show = (feed: StatsFeed) =>
  render(
    <NuqsAdapter>
      <FeedContext.Provider value={feed}>
        <SystemSummary />
      </FeedContext.Provider>
    </NuqsAdapter>,
  );

test("lists_the_three_nearest_to_breaking_highest_first", () => {
  show(
    withNodes([
      busy("a", 0.2, 5000),
      busy("b", 0.9, 1300),
      busy("c", 0.5, 2000),
      busy("d", 0.7, 1800),
    ]),
  );
  const rows = screen.getAllByRole("button").map((b) => b.textContent);
  expect(rows).toEqual(["b90% busy at its peak", "d70% busy at its peak", "c50% busy at its peak"]);
  expect(screen.getByText("Saturates near 1,300 req/s")).toBeTruthy();
  expect(screen.getByText("Select a component to see and change it.")).toBeTruthy();
});

test("says_so_with_no_run_and_with_nothing_under_load", () => {
  show(EMPTY_FEED);
  expect(screen.getByText("Run to find out.")).toBeTruthy();
  cleanup();
  show(withNodes([busy("a", 0, 5000)]));
  expect(screen.getByText("Nothing is under load yet.")).toBeTruthy();
});
