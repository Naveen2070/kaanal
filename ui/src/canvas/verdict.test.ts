import { expect, test } from "vite-plus/test";
import type { Summary } from "../data/api/types";
import { nodeSummary } from "../test/summary";
import { verdictOf } from "./verdict";

const run = (over: Partial<Summary> = {}): Summary => ({
  label: "run",
  window_s: 10,
  requests: 1000,
  failed: 0,
  errors: {},
  warnings: [],
  latency: { p50_ms: 5, p95_ms: 20, p99_ms: 40, max_ms: 90, buckets: [] },
  series: {} as Summary["series"],
  streams: [],
  nodes: [],
  links: [],
  bottlenecks: [],
  ...over,
});

test("a_run_with_no_requests_has_no_verdict", () => {
  expect(verdictOf(run({ requests: 0 }))).toBeNull();
});

test("a_clean_run_says_so_and_names_the_first_limit", () => {
  const s = run({
    nodes: [nodeSummary({ name: "app", saturates_at_req_s: 210, saturated: false })],
  });
  expect(verdictOf(s)).toBe(
    "All 1,000 requests succeeded. app is the first limit: it fills at 210 req/s.",
  );
});

test("a_clean_run_with_no_limit_says_only_that_it_succeeded", () => {
  expect(verdictOf(run())).toBe("All 1,000 requests succeeded.");
});

test("a_failing_run_gives_the_share_and_the_commonest_kind", () => {
  const s = run({
    failed: 740,
    errors: { timeout: 700, status: 40 },
    nodes: [nodeSummary({ name: "app", saturates_at_req_s: 210, saturated: true })],
  });
  expect(verdictOf(s)).toBe(
    "74% of requests failed, mostly timeout. app is full: it fills at 210 req/s.",
  );
});

test("no_kind_is_called_most_when_none_has_over_half", () => {
  const s = run({ failed: 100, errors: { timeout: 40, status: 35, reset: 25 } });
  expect(verdictOf(s)).toBe("10% of requests failed, most often timeout.");
});

test("under_one_percent_is_not_rounded_to_zero", () => {
  expect(verdictOf(run({ failed: 3, errors: { status: 3 } }))).toBe(
    "Less than 1% of requests failed, mostly status.",
  );
});

test("a_wire_can_be_the_first_limit", () => {
  const s = run({
    links: [
      {
        from: "gw",
        to: "app",
        transfers: 1,
        mean_wait_ms: 1,
        offered_load: 2,
        utilization: 1,
        saturated: true,
        saturates_at_req_s: 5,
        series: { window_s: 1, latency: [] },
      },
    ],
  });
  expect(verdictOf(run({ ...s }))).toBe(
    "All 1,000 requests succeeded. gw → app is full: it fills at 5 req/s.",
  );
});
