import { expect, test } from "vite-plus/test";
import type { SweepPointDoc, SweepResultDoc } from "../../data/api/types";
import { describeSweep, finding, logScale, p99Domain, sweepGeometry } from "./sweepMath";

const res = (verdict: SweepResultDoc["verdict"]) =>
  ({
    label: "x",
    limit: { p99_ms: 200, error_rate: 0.01 },
    points: [],
    verdict,
    recovers_at: null,
    stopped_at: null,
    first_to_saturate: null,
    dominant_cause: null,
    warnings: [],
  }) as SweepResultDoc;

const pt = (k: number, p99: number, over = false): SweepPointDoc => ({
  k,
  req_s: 100 * k,
  requests: 1000,
  goodput_req_s: 100 * k,
  error_rate: 0,
  p50_ms: p99 / 4,
  p95_ms: p99 / 2,
  p99_ms: p99,
  cause: null,
  nodes: [],
  over: { p99: over, errors: false },
});

const size = { w: 400, h: 150 };

test("logScale_maps_the_ends_and_the_geometric_middle", () => {
  const s = logScale(1, 100, 0, 200);
  expect(s(1)).toBeCloseTo(0, 9);
  expect(s(100)).toBeCloseTo(200, 9);
  expect(s(10)).toBeCloseTo(100, 9); // the geometric mean sits in the middle of a log axis
});

test("sweepGeometry_puts_over_limit_points_above_the_limit_line_and_gives_a_knee_x", () => {
  const pts = [pt(1, 20), pt(2, 60), pt(4, 400, true), pt(8, 3000, true)];
  const g = sweepGeometry(pts, 200, size);
  expect(g.dots).toHaveLength(4);
  expect(g.dots.map((d) => d.over)).toEqual([false, false, true, true]);
  for (const d of g.dots) {
    if (d.over) expect(d.y).toBeLessThan(g.limitY); // SVG y grows downward
    else expect(d.y).toBeGreaterThan(g.limitY);
  }
  // the knee is the last passing point, the second one
  expect(g.kneeX).toBeCloseTo((g.dots[1] as { x: number }).x, 9);
  expect(g.line.split(" ")).toHaveLength(4);
});

test("an_empty_list_gives_no_dots", () => {
  const g = sweepGeometry([], 200, size);
  expect(g.dots).toEqual([]);
  expect(g.line).toBe("");
  expect(g.kneeX).toBeNull();
});

test("a_single_point_is_drawn", () => {
  const g = sweepGeometry([pt(1, 20)], 200, size);
  expect(g.dots).toHaveLength(1);
  const d = g.dots[0] as { x: number; y: number };
  expect(Number.isFinite(d.x) && Number.isFinite(d.y)).toBe(true);
  expect(g.kneeX).toBeNull(); // nothing crossed
});

test("describeSweep_names_the_knee_the_limit_and_the_first_node", () => {
  const r = {
    label: "x",
    limit: { p99_ms: 200, error_rate: 0.01 },
    points: [],
    verdict: { kind: "knee", k: 2, req_s: 400 },
    recovers_at: null,
    stopped_at: null,
    first_to_saturate: { node: "db", k: 2.5, req_s: 500, analytic_req_s: null },
    dominant_cause: null,
    warnings: [],
  } as SweepResultDoc;
  const s = describeSweep(r);
  expect(s).toContain("knee");
  expect(s).toContain("400 req/s");
  expect(s).toContain("200 ms");
  expect(s).toContain("db");
});

test("unsorted_input_gives_the_same_geometry_as_sorted", () => {
  const sorted = [pt(1, 20), pt(2, 60), pt(4, 400, true)];
  const shuffled = [sorted[2], sorted[0], sorted[1]] as SweepPointDoc[];
  expect(sweepGeometry(shuffled, 200, size)).toEqual(sweepGeometry(sorted, 200, size));
});

test("the_first_point_over_gives_no_knee_x", () => {
  expect(sweepGeometry([pt(1, 400, true), pt(2, 900, true)], 200, size).kneeX).toBeNull();
});

test("a_limit_outside_the_data_range_stays_inside_the_plot", () => {
  for (const limit of [0.5, 100000]) {
    const g = sweepGeometry([pt(1, 20), pt(2, 60)], limit, size);
    expect(g.limitY).toBeGreaterThanOrEqual(0);
    expect(g.limitY).toBeLessThanOrEqual(size.h);
    for (const dot of g.dots) {
      expect(dot.y).toBeGreaterThanOrEqual(0);
      expect(dot.y).toBeLessThanOrEqual(size.h);
    }
  }
});

test("a_zero_p99_with_a_bad_limit_gives_finite_numbers", () => {
  for (const limit of [0, -1, Number.NaN]) {
    const g = sweepGeometry([pt(1, 0)], limit, size);
    expect(Number.isFinite(g.limitY)).toBe(true);
    for (const dot of g.dots) expect(Number.isFinite(dot.x) && Number.isFinite(dot.y)).toBe(true);
    const [lo, hi] = p99Domain([pt(1, 0)], limit);
    expect(Number.isFinite(lo) && Number.isFinite(hi) && lo > 0 && hi > lo).toBe(true);
  }
});

test("a_point_over_on_errors_only_is_over", () => {
  const p = { ...pt(2, 10), over: { p99: false, errors: true } };
  const g = sweepGeometry([pt(1, 10), p], 200, size);
  expect(g.dots.map((x) => x.over)).toEqual([false, true]);
});

test("describeSweep_words_broken_at_lowest_and_holds_to_honestly", () => {
  const b = describeSweep(res({ kind: "broken_at_lowest", k: 1, req_s: 100 }));
  expect(b).toContain("lowest point");
  expect(b).toContain("100 req/s");
  const h = describeSweep(res({ kind: "holds_to", k: 8, req_s: 800 }));
  expect(h).toContain("no failure seen up to 800 req/s");
  expect(h).not.toContain("knee");
  expect(describeSweep(res({ kind: "knee", k: 2, req_s: 400 }))).not.toMatch(/p99 (breach|exceed)/);
});

test("a_zero_or_infinite_p99_among_positive_ones_stays_inside_the_plot", () => {
  for (const bad of [0, Number.POSITIVE_INFINITY]) {
    const g = sweepGeometry([pt(1, bad), pt(2, 20), pt(4, 60)], 200, size);
    for (const dot of g.dots) {
      expect(Number.isFinite(dot.y)).toBe(true);
      expect(dot.y).toBeGreaterThanOrEqual(0);
      expect(dot.y).toBeLessThanOrEqual(size.h);
    }
    for (const xy of g.line.split(" ")) {
      const y = Number(xy.split(",")[1]);
      expect(Number.isFinite(y) && y >= 0 && y <= size.h).toBe(true);
    }
  }
  // a zero p99 sits at the bottom edge
  expect(sweepGeometry([pt(1, 0), pt(2, 20), pt(4, 60)], 200, size).dots[0]?.y).toBe(size.h);
});

test("finding_leaves_the_limit_phrase_out_for_a_bad_limit", () => {
  const v = { kind: "knee", k: 2, req_s: 400 } as const;
  for (const limit of [Number.NaN, 0, -5]) {
    const s = finding(v, limit, null);
    expect(s).not.toContain("NaN");
    expect(s).not.toContain("p99 limit");
    expect(s).toContain("knee");
  }
  expect(finding(v, 200, null)).toContain("p99 limit 200 ms");
});

test("describeSweep_mentions_where_the_ladder_was_cut", () => {
  const r = { ...res({ kind: "holds_to", k: 8, req_s: 800 }), stopped_at: 16 };
  expect(describeSweep(r)).toContain("cut at 16x");
  expect(describeSweep(res({ kind: "holds_to", k: 8, req_s: 800 }))).not.toContain("cut at");
});
