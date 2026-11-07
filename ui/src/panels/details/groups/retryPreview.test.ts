import { expect, test } from "vite-plus/test";
import type { EdgeDoc } from "../../../data/api/types";
import { retryDiff, retryWaits } from "./retryPreview";

const e = (extra: Record<string, unknown> = {}): EdgeDoc => ({ from: "a", to: "b", ...extra });

// Exponential backoff as gRPC A6 and the AWS Architecture Blog write it: the wait before retry n is
// min(cap, initial * multiplier^(n-1)). The expected numbers are worked by hand, not read from the code.
test("waits_double_from_the_first_wait_for_every_retry_after_the_first_try", () => {
  expect(retryWaits({})).toEqual([50, 100]); // 3 tries = 2 retries, defaults 50 ms x2
  expect(retryWaits({ max_attempts: 5, backoff: "100ms", multiplier: 3 })).toEqual([
    100, 300, 900, 2700,
  ]);
});

test("a_wait_never_passes_the_longest_wait", () => {
  expect(retryWaits({ max_attempts: 5, multiplier: 10 })).toEqual([50, 500, 5000, 20000]); // 50000 capped at 20 s
  expect(retryWaits({ max_attempts: 4, backoff: "1s", multiplier: 2, cap: "1500ms" })).toEqual([
    1000, 1500, 1500,
  ]);
});

test("a_multiplier_of_one_keeps_the_wait_constant", () => {
  expect(retryWaits({ max_attempts: 4, multiplier: 1 })).toEqual([50, 50, 50]);
});

test("nothing_differs_on_a_wire_with_no_retry_settings", () => {
  expect(retryDiff(e())).toEqual([]);
});

test("only_rows_that_differ_from_the_default_are_listed", () => {
  const rows = retryDiff(e({ retry: { max_attempts: 5, backoff: "50ms" }, abandon: "continue" }));
  expect(rows).toEqual([
    { label: "Retry", was: "off", now: "on" },
    { label: "Tries", was: "3", now: "5" },
    { label: "When the caller gives up", was: "stops the work", now: "work continues" },
  ]);
});

test("a_budget_and_writes_and_jitter_show_when_set", () => {
  const rows = retryDiff(e({ retry: { budget: {}, writes: true, jitter: "none" } }));
  expect(rows.map((r) => r.label)).toEqual([
    "Retry",
    "Randomise the wait",
    "Retry writes",
    "Budget",
  ]);
  expect(rows.find((r) => r.label === "Budget")).toEqual({
    label: "Budget",
    was: "off",
    now: "on",
  });
});

test("abandon_alone_is_listed_without_a_retry_row", () => {
  expect(retryDiff(e({ abandon: "continue" }))).toEqual([
    { label: "When the caller gives up", was: "stops the work", now: "work continues" },
  ]);
});
