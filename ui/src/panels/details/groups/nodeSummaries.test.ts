import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../../data/api/types";
import {
  balanceSummary,
  limitSummary,
  nodeGroupChanged,
  nodeResetOps,
  styleSummary,
} from "./summaries";

const n = (more: Record<string, unknown> = {}): NodeDoc => ({ type: "service", ...more });

test("a_request_limit_says_its_rate_and_burst_or_none", () => {
  expect(limitSummary(n())).toBe("none");
  expect(limitSummary(n({ rate_limit: { rps: 100, burst: 20 } }))).toBe("100 / s · burst 20");
  expect(limitSummary(n({ rate_limit: { rps: 5500, burst: 5500 } }))).toBe(
    "5,500 / s · burst 5,500",
  );
});

test("a_limit_in_a_shape_the_boxes_cannot_show_is_still_called_set", () => {
  expect(limitSummary(n({ rate_limit: "weird" }))).toBe("set in kaanal.yaml");
});

test("server_style_says_event_loop_or_thread_per_request", () => {
  expect(styleSummary(n())).toBe("event loop");
  expect(styleSummary(n({ hold: "call" }))).toBe("thread per request");
});

test("picking_a_copy_says_the_balance_and_for_a_balancer_the_spread_and_detect", () => {
  expect(balanceSummary(n({ replicas: 3 }))).toBe("round robin");
  expect(balanceSummary(n({ replicas: 3, balance: "p2c" }))).toBe("power of two choices");
  expect(balanceSummary(n({ type: "load-balancer", spread: "random", detect: "10s" }))).toBe(
    "spread random · notices after 10 s",
  );
  expect(balanceSummary(n({ type: "load-balancer" }))).toBe("spread round robin");
});

test("changed_means_a_key_of_the_group_is_written_and_reset_removes_those_keys", () => {
  expect(nodeGroupChanged("limit", n())).toBe(false);
  expect(nodeGroupChanged("limit", n({ rate_limit: { rps: 1, burst: 1 } }))).toBe(true);
  expect(nodeGroupChanged("balance", n({ spread: "random" }))).toBe(true);
  expect(nodeGroupChanged("style", n({ hold: "call" }))).toBe(true);
  expect(nodeResetOps("balance", "lb", n({ spread: "random", detect: "5s", proc: "1ms" }))).toEqual(
    [
      { op: "remove", path: ["nodes", "lb", "spread"] },
      { op: "remove", path: ["nodes", "lb", "detect"] },
    ],
  );
  expect(nodeResetOps("limit", "app", n())).toEqual([]);
});
