import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { Journey, Tick } from "../../data/api/liveTypes";
import { useLive } from "../../data/store/live";
import { MAX_DOTS } from "./dots";
import { useDotFeeder, useDots } from "./dotStore";

const reduced = vi.hoisted(() => ({ value: false }));
vi.mock("../../overlays/first-run/motion", () => ({
  usePrefersReducedMotion: () => reduced.value,
}));

const journey: Journey = {
  hops: [
    {
      node: 1,
      replica: 0,
      kind: "net",
      start_ns: 0,
      end_ns: 100e6,
      wait_ns: 0,
      attempt: 1,
      edge: 0,
    },
  ],
  dropped_at: null,
  total_ns: 100e6,
};
const tick = (t_ms: number, journeys: Journey[]): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: journeys.length,
  failed: 0,
  nodes: [],
  wires: [],
  hist: [],
  journeys,
  status: { rate: 1, paused: false, overloaded: false, in_flight: 0 },
});

beforeEach(() => {
  reduced.value = false;
  useLive.getState().reset();
  useLive.setState({ status: "running" });
  useDots.getState().clear();
});
afterEach(cleanup);

test("a_new_tick_with_journeys_adds_dots_and_the_same_tick_twice_does_not", () => {
  renderHook(() => useDotFeeder());
  act(() => useLive.setState({ ticks: [tick(250, [journey, journey])] }));
  expect(useDots.getState().dots).toHaveLength(2);
  act(() => useLive.setState({ ticks: [tick(250, [journey, journey])] }));
  expect(useDots.getState().dots).toHaveLength(2);
});

test("at_most_sixty_dots_are_on_screen_whatever_the_ticks_bring", () => {
  renderHook(() => useDotFeeder());
  for (let i = 1; i <= 20; i++) {
    act(() => useLive.setState({ ticks: [tick(i * 250, Array(10).fill(journey))] }));
  }
  expect(useDots.getState().dots.length).toBeLessThanOrEqual(MAX_DOTS);
  expect(useDots.getState().dots.length).toBe(MAX_DOTS);
});

test("nothing_is_drawn_while_paused", () => {
  renderHook(() => useDotFeeder());
  useLive.setState({ status: "paused" });
  act(() => useLive.setState({ ticks: [tick(250, [journey])] }));
  expect(useDots.getState().dots).toHaveLength(0);
});

test("reduced_motion_draws_no_dots_and_clears_the_ones_on_screen", () => {
  useDots.getState().add([{ key: "k", edge: 0, delayMs: 0, durMs: 100 }], 0);
  reduced.value = true;
  renderHook(() => useDotFeeder());
  expect(useDots.getState().dots).toHaveLength(0);
  act(() => useLive.setState({ ticks: [tick(250, [journey])] }));
  expect(useDots.getState().dots).toHaveLength(0);
});

test("finished_dots_are_pruned", () => {
  useDots.getState().add([{ key: "k", edge: 0, delayMs: 0, durMs: 100 }], 1000);
  useDots.getState().prune(1050);
  expect(useDots.getState().dots).toHaveLength(1);
  useDots.getState().prune(1300);
  expect(useDots.getState().dots).toHaveLength(0);
});
