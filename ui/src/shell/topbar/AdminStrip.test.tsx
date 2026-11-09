import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { Tick } from "../../data/api/liveTypes";
import type { DraftView } from "../../data/api/types";
import { useAdmin } from "../../data/store/admin";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { bucketOf } from "../../data/store/liveMath";
import { expectNoViolations } from "../../test/axe";
import { AdminStrip } from "./AdminStrip";

vi.mock("../../data/store/liveRuntime", () => ({
  pauseLive: vi.fn(() => Promise.resolve()),
  resumeLive: vi.fn(() => Promise.resolve()),
}));

const none = { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0 };
const node = { arrived: 10, dropped: none, queue: 0, busy_slots: 0, cpu_load: null };
/** 25 requests a tick: 5 real (slow), 20 generated (fast). */
const tick = (t_ms: number): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: 25,
  failed: 0,
  nodes: [node],
  wires: [],
  hist: [
    [bucketOf(10e6), 20],
    [bucketOf(800e6), 5],
  ],
  journeys: [],
  status: { rate: 100, paused: false, overloaded: false, in_flight: 0 },
  real: { requests: 5, failed: 0, nodes: [node], wires: [], hist: [[bucketOf(800e6), 5]] },
});

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  useDraft.setState({
    view: { doc: { nodes: { app: { type: "service" } }, edges: [] } } as unknown as DraftView,
  });
  useLive.getState().reset();
  useAdmin.getState().reset();
  act(() => {
    for (let i = 1; i <= 8; i++) useLive.getState().ingest({ type: "tick", tick: tick(i * 250) });
    useLive.setState({ status: "running" });
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

test("it_shows_the_live_figures_and_the_real_and_generated_split", () => {
  render(<AdminStrip />);
  // 100 requests a second in all: 20 real, 80 generated
  expect(screen.getByLabelText("Real and generated requests").textContent).toMatch(
    /Real\s*20 \/ s.*Generated\s*80 \/ s/s,
  );
  expect(screen.getByLabelText("Live figures")).toBeTruthy();
});

test("real_only_switches_the_strip_figures_to_the_real_requests", () => {
  render(<AdminStrip />);
  const arriving = () => screen.getByText("Arriving").nextElementSibling?.textContent;
  expect(arriving()).toBe("100 / s");
  const toggle = screen.getByRole("button", { name: "Real only" });
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  expect(useAdmin.getState().realOnly).toBe(true);
  expect(arriving()).toBe("20 / s");
});

test("it_says_so_when_no_real_request_has_arrived", () => {
  act(() => {
    useLive.getState().reset();
    for (let i = 1; i <= 4; i++) {
      const t = tick(i * 250);
      delete t.real;
      useLive.getState().ingest({ type: "tick", tick: t });
    }
  });
  render(<AdminStrip />);
  expect(screen.getByLabelText("Real and generated requests").textContent).toMatch(
    /No real requests yet/,
  );
});

test("has_no_axe_violations", async () => {
  const { container } = render(<AdminStrip />);
  await expectNoViolations(container);
});
