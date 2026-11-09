import { expect, test } from "vite-plus/test";
import type { Journey, JourneyHop } from "../../data/api/liveTypes";
import { dotDone, MAX_DOTS, pileSize, planDots, withCap, type Dot } from "./dots";

const hop = (over: Partial<JourneyHop>): JourneyHop => ({
  node: 1,
  replica: 0,
  kind: "proc",
  start_ns: 0,
  end_ns: 1_000_000,
  wait_ns: 0,
  attempt: 1,
  edge: null,
  ...over,
});
const journey = (hops: JourneyHop[], dropped_at: number | null = null): Journey => ({
  hops,
  dropped_at,
  total_ns: 1,
});

test("a_slow_wire_makes_a_slow_dot_and_a_fast_one_is_still_visible", () => {
  const slow = planDots([journey([hop({ kind: "net", edge: 0, start_ns: 0, end_ns: 900e6 })])], 1);
  const fast = planDots([journey([hop({ kind: "net", edge: 0, start_ns: 0, end_ns: 1e6 })])], 1);
  expect(slow[0]?.durMs).toBe(900);
  expect(fast[0]?.durMs).toBe(160); // never so quick it cannot be seen
  const hung = planDots([journey([hop({ kind: "net", edge: 0, end_ns: 60_000e6 })])], 1);
  expect(hung[0]?.durMs).toBe(4000);
});

test("a_dot_starts_when_its_hop_starts", () => {
  const d = planDots([journey([hop({ kind: "net", edge: 2, start_ns: 400e6, end_ns: 600e6 })])], 7);
  expect(d).toEqual([{ key: "7-0-0", edge: 2, delayMs: 400, durMs: 200 }]);
});

test("an_own_hop_entered_over_a_wire_without_latency_still_gets_a_dot_that_ends_at_its_start", () => {
  const d = planDots([journey([hop({ edge: 1, start_ns: 1000e6, end_ns: 1010e6 })])], 3);
  expect(d).toEqual([{ key: "3-0-0", edge: 1, delayMs: 700, durMs: 300 }]);
});

test("a_net_hop_and_the_own_hop_after_it_on_the_same_wire_are_one_dot", () => {
  const d = planDots(
    [
      journey([
        hop({ kind: "net", edge: 0, start_ns: 0, end_ns: 50e6 }),
        hop({ kind: "proc", edge: 0, start_ns: 50e6, end_ns: 60e6 }),
      ]),
    ],
    1,
  );
  expect(d).toHaveLength(1);
});

test("hops_without_a_wire_and_hops_far_in_the_future_make_no_dot", () => {
  const d = planDots(
    [
      journey([
        hop({ edge: null }),
        hop({ kind: "net", edge: 0, start_ns: 9000e6, end_ns: 9100e6 }),
      ]),
    ],
    1,
  );
  expect(d).toEqual([]);
});

test("at_most_sixty_dots_stay_and_the_newest_win", () => {
  const mk = (n: number, tag: string): Dot[] =>
    Array.from({ length: n }, (_, i) => ({ key: `${tag}${i}`, edge: 0, delayMs: 0, durMs: 100 }));
  const all = withCap(mk(50, "a"), mk(30, "b"));
  expect(all).toHaveLength(MAX_DOTS);
  expect(all.at(-1)?.key).toBe("b29");
  expect(all[0]?.key).toBe("a20");
  expect(withCap([], mk(3, "c"))).toHaveLength(3);
});

test("a_dot_is_done_after_its_delay_and_travel", () => {
  const d: Dot = { key: "k", edge: 0, delayMs: 200, durMs: 300 };
  expect(dotDone(d, 500)).toBe(false);
  expect(dotDone(d, 700)).toBe(true);
});

test("the_pile_is_the_queue_up_to_twelve", () => {
  expect([0, 3, 12, 400, -5, 2.9].map(pileSize)).toEqual([0, 3, 12, 12, 0, 2]);
});

test("a_real_journey_makes_real_dots_and_a_generated_one_has_no_real_key", () => {
  const hops = [hop({ kind: "net", edge: 0, start_ns: 0, end_ns: 200e6 })];
  const real = planDots([{ ...journey(hops), real: true }], 1);
  const gen = planDots([journey(hops)], 1);
  expect(real[0]?.real).toBe(true);
  expect("real" in (gen[0] as object)).toBe(false);
});
