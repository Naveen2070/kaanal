import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import type { SweepPointDoc } from "../../data/api/types";
import { expectNoViolations } from "../../test/axe";
import { SweepChart } from "./SweepChart";

const pt = (k: number, p99: number, over: boolean): SweepPointDoc => ({
  k,
  req_s: 100 * k,
  requests: 1000,
  goodput_req_s: 100 * k,
  error_rate: over ? 0.05 : 0,
  p50_ms: 5,
  p95_ms: 10,
  p99_ms: p99,
  cause: null,
  nodes: [],
  over: { p99: over, errors: false },
});
const points = [pt(1, 20, false), pt(2, 60, false), pt(4, 400, true)];
const knee = { kind: "knee", k: 2, req_s: 200 } as const;
const first = { node: "db", k: 4, req_s: 400, analytic_req_s: null };

afterEach(cleanup);

test("the_chart_has_an_img_role_a_label_and_a_table", async () => {
  const { container } = render(
    <SweepChart points={points} limitMs={200} knee={knee} firstToSaturate={first} />,
  );
  const img = screen.getByRole("img", { name: /knee at 200 req\/s/ });
  expect(img.getAttribute("aria-label")).toContain("db");
  expect(screen.getByText(/limit p99 200 ms/)).toBeTruthy();
  expect(screen.getByText(/knee/, { selector: "text" })).toBeTruthy();
  expect(container.querySelector("details table")).not.toBeNull();
  expect(container.querySelectorAll("tbody tr")).toHaveLength(3);
  await expectNoViolations(container);
});

test("over_limit_points_use_a_different_shape", () => {
  const { container } = render(
    <SweepChart points={points} limitMs={200} knee={knee} firstToSaturate={null} />,
  );
  expect(container.querySelectorAll("circle[data-dot]")).toHaveLength(2);
  expect(container.querySelectorAll("rect[data-dot]")).toHaveLength(1);
});

test("an_empty_chart_renders_without_axe_violations", async () => {
  const { container } = render(
    <SweepChart points={[]} limitMs={200} knee={null} firstToSaturate={null} />,
  );
  expect(container.querySelector("svg")).not.toBeNull();
  await expectNoViolations(container);
});

test("a_zero_p99_with_a_zero_limit_renders_without_hanging_or_violations", async () => {
  for (const limit of [0, Number.NaN]) {
    const { container } = render(
      <SweepChart points={[pt(1, 0, false)]} limitMs={limit} knee={null} firstToSaturate={null} />,
    );
    expect(container.querySelectorAll("[data-dot]")).toHaveLength(1);
    await expectNoViolations(container);
    cleanup();
  }
}, 5000);

test("only_a_knee_verdict_draws_the_knee_line", () => {
  for (const kind of ["holds_to", "broken_at_lowest"] as const) {
    const { container } = render(
      <SweepChart
        points={points}
        limitMs={200}
        knee={{ kind, k: 2, req_s: 200 }}
        firstToSaturate={null}
      />,
    );
    expect(container.querySelector("line.stroke-accent")).toBeNull();
    cleanup();
  }
  const { container } = render(
    <SweepChart points={points} limitMs={200} knee={knee} firstToSaturate={null} />,
  );
  expect(container.querySelector("line.stroke-accent")).not.toBeNull();
});

test("a_legend_explains_the_marker_shapes", () => {
  render(<SweepChart points={points} limitMs={200} knee={knee} firstToSaturate={null} />);
  expect(screen.getByText(/square: over a limit/)).toBeTruthy();
});

test("duplicate_load_factors_render_without_key_warnings", () => {
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  try {
    const { container } = render(
      <SweepChart
        points={[pt(1, 20, false), pt(1, 30, false)]}
        limitMs={200}
        knee={null}
        firstToSaturate={null}
      />,
    );
    expect(container.querySelectorAll("[data-dot]")).toHaveLength(2);
    expect(err.mock.calls.filter((c) => String(c[0]).includes("same key"))).toEqual([]);
  } finally {
    err.mockRestore();
  }
});

test("non_finite_and_negative_error_rates_give_no_bad_svg_heights", () => {
  const bad = [
    { ...pt(1, 20, false), error_rate: Number.NaN },
    { ...pt(2, 30, false), error_rate: -0.5 },
    { ...pt(4, 40, false), error_rate: Number.POSITIVE_INFINITY },
    { ...pt(8, 50, false), error_rate: 0.02 },
  ];
  const { container } = render(
    <SweepChart points={bad} limitMs={200} knee={null} firstToSaturate={null} />,
  );
  const hs = [...container.querySelectorAll("rect.fill-negative:not([data-dot])")].map((r) =>
    Number(r.getAttribute("height")),
  );
  expect(hs).toHaveLength(4);
  for (const h of hs) expect(Number.isFinite(h) && h >= 0).toBe(true);
  expect(container.querySelector("svg")?.innerHTML).not.toContain("NaN");
});

test("table_rows_and_error_bars_follow_the_sorted_order", () => {
  const shuffled = [
    { ...pt(4, 400, true), error_rate: 0.04 },
    { ...pt(1, 20, false), error_rate: 0.01 },
    { ...pt(2, 60, false), error_rate: 0.02 },
  ];
  const { container } = render(
    <SweepChart points={shuffled} limitMs={200} knee={null} firstToSaturate={null} />,
  );
  const loads = [...container.querySelectorAll("tbody tr td:first-child")].map(
    (c) => c.textContent,
  );
  expect(loads).toEqual(["1x", "2x", "4x"]);
  const hs = [...container.querySelectorAll("rect.fill-negative:not([data-dot])")].map((r) =>
    Number(r.getAttribute("height")),
  );
  expect(hs[0]).toBeLessThan(hs[1] as number);
  expect(hs[1]).toBeLessThan(hs[2] as number);
});
