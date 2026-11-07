import { expect, test } from "vite-plus/test";
import type { EdgeDoc } from "../../../data/api/types";
import {
  breakerSummary,
  compressStreamSummary,
  connectionSummary,
  GROUP_KEYS,
  humanMs,
  resetOps,
  retrySummary,
  usageSummary,
  wireGroupChanged,
} from "./summaries";

const e = (more: Record<string, unknown> = {}): EdgeDoc => ({ from: "a", to: "b", ...more });

test("humanMs_reads_like_a_person_would_say_it", () => {
  expect(humanMs(50)).toBe("50 ms");
  expect(humanMs(0.5)).toBe("0.5 ms");
  expect(humanMs(1000)).toBe("1 s");
  expect(humanMs(20_000)).toBe("20 s");
  expect(humanMs(1500)).toBe("1.5 s");
  expect(humanMs(120_000)).toBe("2 min");
});

test("a_wire_with_nothing_set_says_defaults_or_off", () => {
  expect(connectionSummary(e())).toBe("defaults");
  expect(compressStreamSummary(e())).toBe("defaults");
  expect(usageSummary(e())).toBe("defaults");
  expect(retrySummary(e())).toBe("off");
  expect(breakerSummary(e())).toBe("off");
});

test("retry_says_tries_wait_growth_and_budget_with_the_loaders_defaults", () => {
  expect(retrySummary(e({ retry: {} }))).toBe("3 tries · 50 ms ×2");
  expect(retrySummary(e({ retry: { max_attempts: 5, backoff: "100ms", multiplier: 3 } }))).toBe(
    "5 tries · 100 ms ×3",
  );
  expect(retrySummary(e({ retry: { budget: {} } }))).toBe("3 tries · 50 ms ×2 · budget on");
  expect(retrySummary(e({ retry: {}, abandon: "continue" }))).toBe(
    "3 tries · 50 ms ×2 · work continues",
  );
});

test("abandon_alone_without_retries_is_still_said", () => {
  expect(retrySummary(e({ abandon: "continue" }))).toBe("retries off · work continues");
});

test("breaker_says_its_window_rate_and_wait", () => {
  expect(breakerSummary(e({ breaker: {} }))).toBe("100 calls · opens at 50% · 60 s");
  expect(breakerSummary(e({ breaker: { window: 50, open_for: "10s", failure_rate: 0.25 } }))).toBe(
    "50 calls · opens at 25% · 10 s",
  );
});

test("connection_lists_only_what_is_set", () => {
  expect(connectionSummary(e({ tls: 1.3 }))).toBe("TLS 1.3");
  expect(connectionSummary(e({ tls: true, keepalive: 1 }))).toBe("TLS 1.3 · 1 call a connection");
  expect(connectionSummary(e({ loss: 0.01 }))).toBe("1% loss");
  expect(connectionSummary(e({ keepalive: 50 }))).toBe("50 calls a connection");
});

test("compress_and_stream_list_only_what_is_set", () => {
  expect(compressStreamSummary(e({ compress: 0.3 }))).toBe("keeps 30%");
  expect(compressStreamSummary(e({ stream: false }))).toBe("buffered");
  expect(compressStreamSummary(e({ compress: 0.3, stream: true }))).toBe("keeps 30% · streamed");
});

test("usage_lists_calls_share_balance_and_detect_when_set", () => {
  expect(usageSummary(e({ calls: 3 }))).toBe("3 calls a request");
  expect(usageSummary(e({ prob: 0.5 }))).toBe("50% of requests");
  expect(usageSummary(e({ calls: 2, detect: "10s" }))).toBe(
    "2 calls a request · notices after 10 s",
  );
});

test("changed_means_a_key_of_the_group_is_in_the_file", () => {
  expect(wireGroupChanged("retry", e())).toBe(false);
  expect(wireGroupChanged("retry", e({ retry: {} }))).toBe(true);
  expect(wireGroupChanged("retry", e({ abandon: "continue" }))).toBe(true);
  expect(wireGroupChanged("breaker", e({ retry: {} }))).toBe(false);
  expect(wireGroupChanged("connection", e({ tls: 1.2 }))).toBe(true);
});

test("reset_removes_exactly_the_groups_keys_that_are_present", () => {
  expect(resetOps("retry", 4, e({ retry: {}, abandon: "continue", loss: 0.01 }))).toEqual([
    { op: "remove", path: ["edges", 4, "retry"] },
    { op: "remove", path: ["edges", 4, "abandon"] },
  ]);
  expect(resetOps("breaker", 0, e())).toEqual([]);
  expect(GROUP_KEYS.connection).toEqual(["tls", "keepalive", "loss"]);
});
