import { expect, test } from "vite-plus/test";
import type { EdgeDoc, NodeDoc } from "../../data/api/types";
import {
  abandonOf,
  attemptsOf,
  backoffMsOf,
  breakerOf,
  budgetOf,
  budgetRatioOf,
  budgetTokensOf,
  capMsOf,
  clearDetectEdge,
  clearDetectNode,
  detectMsOf,
  jitterOf,
  minCallsOf,
  multiplierOf,
  openForMsOf,
  probesOf,
  ratePctOf,
  retryOf,
  setAbandon,
  setBreaker,
  setBreakerOn,
  setBudget,
  setBudgetOn,
  setDetectEdge,
  setDetectNode,
  setJitter,
  setOpenFor,
  setRatePct,
  setRetry,
  setRetryMs,
  setRetryOn,
  setWrites,
  windowOf,
  writesOf,
} from "./resilience";

const plain: EdgeDoc = { from: "app", to: "db" };

test("a_wire_with_nothing_written_reads_as_off_with_the_loaders_defaults", () => {
  expect(retryOf(plain)).toBeNull();
  expect(breakerOf(plain)).toBeNull();
  expect(abandonOf(plain)).toBe("cancel");
  expect(detectMsOf(plain)).toBeNull();
  const r = {};
  expect([attemptsOf(r), multiplierOf(r), jitterOf(r), writesOf(r)]).toEqual([3, 2, "full", false]);
  expect([backoffMsOf(r), capMsOf(r)]).toEqual([null, null]);
  const b = {};
  expect([windowOf(b), minCallsOf(b), ratePctOf(b), probesOf(b)]).toEqual([100, 100, 50, 10]);
  expect(openForMsOf(b)).toBe(60_000);
  expect([budgetTokensOf(b), budgetRatioOf(b)]).toEqual([10, 0.1]);
});

test("retry_on_writes_the_defaults_and_off_removes_the_key", () => {
  expect(setRetryOn(1, plain, true)).toEqual([
    { op: "insert", path: ["edges", 1, "retry"], value: {} },
  ]);
  expect(setRetryOn(1, { ...plain, retry: {} }, true)).toEqual([]);
  expect(setRetryOn(1, { ...plain, retry: { max_attempts: 4 } }, false)).toEqual([
    { op: "remove", path: ["edges", 1, "retry"] },
  ]);
  expect(setRetryOn(1, plain, false)).toEqual([]);
});

test("a_retry_field_inserts_when_new_and_sets_when_written", () => {
  expect(setRetry(1, {}, "max_attempts", 4)).toEqual({
    op: "insert",
    path: ["edges", 1, "retry", "max_attempts"],
    value: 4,
  });
  expect(setRetry(1, { max_attempts: 3 }, "max_attempts", 4).op).toBe("set");
  expect(setRetryMs(1, {}, "backoff", 100)).toMatchObject({ value: "100ms" });
  expect(backoffMsOf({ backoff: "2s" })).toBe(2000);
});

test("jitter_and_writes_remove_their_key_at_the_default", () => {
  expect(setJitter(1, { jitter: "none" }, "full")).toEqual([
    { op: "remove", path: ["edges", 1, "retry", "jitter"] },
  ]);
  expect(setJitter(1, {}, "full")).toEqual([]);
  expect(setJitter(1, {}, "equal")).toEqual([
    { op: "insert", path: ["edges", 1, "retry", "jitter"], value: "equal" },
  ]);
  expect(setWrites(1, { writes: true }, false)).toEqual([
    { op: "remove", path: ["edges", 1, "retry", "writes"] },
  ]);
  expect(setWrites(1, {}, true)[0]?.op).toBe("insert");
});

test("the_budget_lives_inside_retry", () => {
  expect(budgetOf({})).toBeNull();
  expect(budgetOf({ budget: {} })).toEqual({});
  expect(setBudgetOn(1, {}, true)).toEqual([
    { op: "insert", path: ["edges", 1, "retry", "budget"], value: {} },
  ]);
  expect(setBudgetOn(1, { budget: {} }, false)).toEqual([
    { op: "remove", path: ["edges", 1, "retry", "budget"] },
  ]);
  expect(setBudget(1, {}, "token_ratio", 0.2)).toEqual({
    op: "insert",
    path: ["edges", 1, "retry", "budget", "token_ratio"],
    value: 0.2,
  });
});

test("abandon_continue_is_written_and_cancel_removes_it", () => {
  expect(setAbandon(1, plain, "continue")).toEqual([
    { op: "insert", path: ["edges", 1, "abandon"], value: "continue" },
  ]);
  expect(setAbandon(1, { ...plain, abandon: "continue" }, "cancel")).toEqual([
    { op: "remove", path: ["edges", 1, "abandon"] },
  ]);
  expect(setAbandon(1, plain, "cancel")).toEqual([]);
  expect(abandonOf({ ...plain, abandon: "continue" })).toBe("continue");
});

test("the_breaker_failure_rate_is_a_percent_in_the_box_and_a_fraction_in_the_file", () => {
  expect(ratePctOf({ failure_rate: 0.25 })).toBe(25);
  expect(setRatePct(1, {}, 25)).toEqual({
    op: "insert",
    path: ["edges", 1, "breaker", "failure_rate"],
    value: 0.25,
  });
  expect(setBreakerOn(1, plain, true)).toEqual([
    { op: "insert", path: ["edges", 1, "breaker"], value: {} },
  ]);
  expect(setBreakerOn(1, { ...plain, breaker: {} }, false)[0]?.op).toBe("remove");
  expect(setBreaker(1, { window: 50 }, "window", 20).op).toBe("set");
  expect(setOpenFor(1, {}, 30_000)).toMatchObject({ value: "30000ms" });
  expect(openForMsOf({ open_for: "5s" })).toBe(5000);
});

test("detect_is_a_duration_on_a_wire_or_a_balancer_and_clearing_removes_it", () => {
  expect(setDetectEdge(1, plain, 5000)).toEqual({
    op: "insert",
    path: ["edges", 1, "detect"],
    value: "5000ms",
  });
  expect(detectMsOf({ detect: "10s" })).toBe(10_000);
  expect(clearDetectEdge(1, { ...plain, detect: "5s" })).toEqual([
    { op: "remove", path: ["edges", 1, "detect"] },
  ]);
  const lb: NodeDoc = { type: "load-balancer" };
  expect(setDetectNode("lb", lb, 0)).toEqual({
    op: "insert",
    path: ["nodes", "lb", "detect"],
    value: "0ms",
  });
  expect(clearDetectNode("lb", { ...lb, detect: "5s" })).toEqual([
    { op: "remove", path: ["nodes", "lb", "detect"] },
  ]);
  expect(clearDetectNode("lb", lb)).toEqual([]);
});
