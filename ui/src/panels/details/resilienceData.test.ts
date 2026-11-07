import { expect, test } from "vite-plus/test";
import type { BreakerSummary, RetrySummary } from "../../data/api/types";
import { bandsOf, funnelOf, wireEffects } from "./resilienceData";

const r = (over: Partial<RetrySummary> = {}): RetrySummary => ({
  from: "gw",
  to: "app",
  calls: 100,
  retried: 40,
  retries: 70,
  recovered: 25,
  gave_up: 15,
  failed: 20,
  ...over,
});

const br = (over: Partial<BreakerSummary> = {}): BreakerSummary => ({
  from: "gw",
  to: "app",
  replica: 0,
  state_at_start: "closed",
  changes: [],
  open_ms: 0,
  ...over,
});

test("the_funnel_splits_every_call_into_four_ends_that_add_up", () => {
  // 100 calls: 25 retried and recovered, 15 retried and gave up, 5 failed without a retry (20 failed
  // in all, 15 of them after retrying), so 55 went through first time
  expect(funnelOf(r())).toEqual([
    { key: "first", label: "Went through first time", count: 55 },
    { key: "recovered", label: "Recovered by a retry", count: 25 },
    { key: "gave_up", label: "Retried, then gave up", count: 15 },
    { key: "failed", label: "Failed without a retry", count: 5 },
  ]);
});

test("calls_still_in_flight_never_make_a_segment_negative", () => {
  const f = funnelOf(r({ calls: 10, retried: 8, recovered: 5, gave_up: 3, failed: 9 }));
  expect(f.every((s) => s.count >= 0)).toBe(true);
  expect(f.find((s) => s.key === "first")?.count).toBe(0);
});

test("wire_effects_are_shares_of_calls_and_of_retried_calls", () => {
  const e = wireEffects(r(), undefined, 30_000);
  expect(e).toEqual({ failedPct: 20, recoveredPct: 63, retriesPerCall: 0.7, openPct: null });
});

test("a_wire_with_no_retried_calls_has_no_recovery_share", () => {
  expect(
    wireEffects(r({ retried: 0, recovered: 0, gave_up: 0, retries: 0 }), undefined, 1000),
  ).toEqual({
    failedPct: 20,
    recoveredPct: null,
    retriesPerCall: 0,
    openPct: null,
  });
});

test("a_breaker_adds_the_share_of_the_window_it_was_open", () => {
  expect(wireEffects(r(), br({ open_ms: 7500 }), 30_000)?.openPct).toBe(25);
});

test("no_calls_means_no_effects", () => {
  expect(wireEffects(r({ calls: 0 }), undefined, 1000)).toBeNull();
});

test("the_bands_run_from_the_start_state_through_each_change_to_the_end", () => {
  const b = br({
    changes: [
      { t_ms: 3000, state: "open" },
      { t_ms: 13000, state: "half_open" },
      { t_ms: 13500, state: "closed" },
    ],
  });
  expect(bandsOf(b, 20_000)).toEqual([
    { state: "closed", from: 0, to: 3000 },
    { state: "open", from: 3000, to: 13000 },
    { state: "half_open", from: 13000, to: 13500 },
    { state: "closed", from: 13500, to: 20000 },
  ]);
});

test("a_breaker_open_at_the_start_begins_with_an_open_band", () => {
  const b = br({ state_at_start: "open", changes: [{ t_ms: 500, state: "half_open" }] });
  expect(bandsOf(b, 1000)).toEqual([
    { state: "open", from: 0, to: 500 },
    { state: "half_open", from: 500, to: 1000 },
  ]);
});

test("a_change_past_the_window_end_is_clipped", () => {
  const b = br({ changes: [{ t_ms: 5000, state: "open" }] });
  expect(bandsOf(b, 4000)).toEqual([{ state: "closed", from: 0, to: 4000 }]);
});
