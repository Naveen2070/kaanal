import { cleanup, render } from "@testing-library/react";
import type { EdgeProps } from "@xyflow/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { nodeSummary } from "../test/summary";
import { FeedContext, type StatsFeed } from "./feed";
import { LoadEdge } from "./LoadEdge";

vi.mock("@xyflow/react", () => ({
  BaseEdge: (p: { path: string; className?: string; style?: object }) => (
    <path data-testid="edge" d={p.path} className={p.className} style={p.style} />
  ),
  getSmoothStepPath: (o: { borderRadius: number }) => [`M0 0 r${o.borderRadius}`, 0, 0],
}));

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const feed = (busyFrom: number, busyTo: number, visits = 100): StatsFeed => ({
  node: (name) =>
    name === "gw"
      ? nodeSummary({ name: "gw", utilization: busyFrom, visits_per_s: 100 })
      : name === "app"
        ? nodeSummary({ name: "app", utilization: busyTo, visits_per_s: visits })
        : undefined,
  dimmed: false,
  peakVisits: 100,
});

function edge(f: StatsFeed) {
  const props = {
    id: "e0",
    sourceX: 0,
    sourceY: 0,
    targetX: 10,
    targetY: 10,
    sourcePosition: "right",
    targetPosition: "left",
    data: { from: "gw", to: "app" },
  };
  render(
    <svg>
      <FeedContext.Provider value={f}>
        <LoadEdge {...(props as unknown as EdgeProps)} />
      </FeedContext.Provider>
    </svg>,
  );
  return document.querySelector("[data-testid=edge]") as SVGPathElement;
}

test("the_path_has_six_pixel_rounded_elbows", () => {
  expect(edge(feed(0.1, 0.1)).getAttribute("d")).toBe("M0 0 r6");
});

test("a_busy_edge_is_thicker_amber_or_red_and_flows", () => {
  const calm = edge(feed(0.1, 0.1, 10));
  expect(calm.style.strokeWidth).toBe("1.8");
  expect(calm.style.stroke).toBe("var(--control)");
  cleanup();
  const hot = edge(feed(0.95, 0.2, 100));
  expect(hot.style.strokeWidth).toBe("4.5");
  expect(hot.style.stroke).toBe("var(--bad-fill)");
  expect(hot.getAttribute("class")).toMatch(/kaanal-flow/);
});

test("an_edge_with_no_traffic_does_not_flow", () => {
  const e = edge(feed(0.1, 0.1, 0));
  expect(e.getAttribute("class") ?? "").not.toMatch(/kaanal-flow/);
});

test("edge_flow_stops_under_reduced_motion", () => {
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: q.includes("reduce"),
    addEventListener() {},
    removeEventListener() {},
  }));
  const e = edge(feed(0.9, 0.9, 100));
  expect(e.getAttribute("class") ?? "").not.toMatch(/kaanal-flow/);
  expect(e.style.stroke).toBe("var(--bad-fill)"); // the state is still in the colour
});

test("an_edge_with_no_run_is_the_muted_default", () => {
  const e = edge({ node: () => undefined, dimmed: false, peakVisits: 0 });
  expect(e.style.strokeWidth).toBe("1.5");
  expect(e.style.stroke).toBe("var(--control)");
});
