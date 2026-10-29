import { expect, test } from "vite-plus/test";
import { nodeSummary } from "../test/summary";
import { nodeFigures, percent, twoSignificant } from "./metrics";

test("figures_take_the_peak_from_the_busy_series", () => {
  const f = nodeFigures(nodeSummary(), { type: "service" });
  expect(f.peak).toBe(0.9);
  expect(f.busy).toBe(0.5);
  expect(f.queue).toEqual([0, 3, 1]);
});

test("headroom_text_rounds_to_two_significant_digits", () => {
  const text = (n: number) =>
    nodeFigures(nodeSummary({ saturates_at_req_s: n }), { type: "service" }).headroomText;
  expect(text(122.4)).toBe("saturates near 120 req/s");
  expect(text(1530)).toBe("saturates near 1,500 req/s");
  expect(text(7.46)).toBe("saturates near 7.5 req/s");
  expect(twoSignificant(0.043)).toBe("0.043");
});

test("a_node_with_no_slots_shows_no_bar_and_no_headroom", () => {
  const f = nodeFigures(
    nodeSummary({
      utilization: null,
      offered_load: null,
      capacity_per_s: null,
      saturates_at_req_s: null,
      headroom: null,
      series: { window_s: 1, busy: [], queue: [0, 0], latency: [] },
    }),
    { type: "gateway" },
  );
  expect(f.busy).toBeNull();
  expect(f.peak).toBeNull();
  expect(f.headroomText).toBeNull();
});

test("a_node_with_no_run_has_no_figures_at_all", () => {
  const f = nodeFigures(undefined, { type: "service", replicas: 3 });
  expect(f).toMatchObject({ busy: null, peak: null, waiting: null, hasRun: false, replicas: 3 });
  expect(percent(f.busy)).toBe("–");
});

test("replicas_default_to_one_and_ignore_nonsense", () => {
  for (const r of [undefined, 0, -2, 1.5, "3", null]) {
    expect(nodeFigures(undefined, { type: "service", replicas: r }).replicas).toBe(1);
  }
});
