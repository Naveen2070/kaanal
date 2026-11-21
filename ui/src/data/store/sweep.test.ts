import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { mockFetch } from "../../test/fetch";
import { TOKEN_KEY } from "../../token";
import type { SweepPointDoc } from "../api/types";
import { ladderSize, useSweep } from "./sweep";

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  vi.useFakeTimers();
});
afterEach(() => {
  useSweep.getState().reset();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

const pt = (k: number): SweepPointDoc => ({
  k,
  req_s: k * 100,
  requests: 1000,
  goodput_req_s: k * 100,
  error_rate: 0,
  p50_ms: 1,
  p95_ms: 2,
  p99_ms: 3,
  cause: null,
  nodes: [],
  over: { p99: false, errors: false },
});

const rec = (state: string, extra: object = {}) => ({
  id: "s1",
  kind: "sweep",
  state,
  generation: 1,
  profile: "peak",
  seed: 7,
  max_factor: 8,
  fingerprint: "abc",
  sweep_points: [],
  ...extra,
});

const advance = (ms: number) => vi.advanceTimersByTimeAsync(ms);

test("the_store_collects_points_then_the_report_once", async () => {
  const sent = mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("running", { sweep_points: [pt(0.25)] }) },
    { json: rec("running", { sweep_points: [pt(0.25), pt(0.35)] }) },
    { json: rec("done", { sweep: { points: [pt(0.25), pt(0.35), pt(0.5)] } }) },
    { text: "# report" },
  ]);
  const p = useSweep.getState().start({ profile: "peak" });
  await advance(0);
  expect(useSweep.getState().points).toHaveLength(1);
  await advance(500);
  expect(useSweep.getState().points).toHaveLength(2);
  await advance(500);
  await p;
  const s = useSweep.getState();
  expect(s.status).toBe("done");
  expect(s.points).toHaveLength(3);
  expect(s.report).toBe("# report");
  await advance(5000);
  expect(sent.filter((r) => r.url.endsWith("/report"))).toHaveLength(1);
  expect(sent).toHaveLength(5);
});

test("a_failed_sweep_keeps_its_error_and_its_points", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("failed", { sweep_points: [pt(0.25)], error: "too large" }) },
  ]);
  await useSweep.getState().start({ profile: "peak" });
  const s = useSweep.getState();
  expect(s.status).toBe("failed");
  expect(s.error).toBe("too large");
  expect(s.points).toHaveLength(1);
  expect(s.report).toBeNull();
});

test("a_busy_start_is_a_result_not_an_error", async () => {
  mockFetch([{ status: 409, json: { error: "a run is in progress" } }]);
  await useSweep.getState().start({ profile: "peak" });
  expect(useSweep.getState().status).toBe("busy");
  expect(useSweep.getState().error).toBe("a run is in progress");
});

test("reset_stops_polling", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s1" } }, { json: rec("running") }]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0);
  expect(useSweep.getState().record).not.toBeNull();
  const before = sent.length;
  useSweep.getState().reset();
  await advance(5000);
  expect(sent).toHaveLength(before);
  expect(useSweep.getState().status).toBe("idle");
});

test("maxPoints_follows_the_records_own_max_factor", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("running", { max_factor: 1, sweep_points: [pt(0.25)] }) },
  ]);
  const p = useSweep.getState().start({ profile: "peak" });
  // until the first poll only the request says: no max_factor is the server's 8
  expect(useSweep.getState().maxPoints).toBe(ladderSize(8) + 6);
  await p;
  expect(useSweep.getState().maxPoints).toBe(ladderSize(1) + 6);
});

test("start_keeps_what_it_sent_and_the_boot", () => {
  mockFetch([{ status: 202, json: { id: "s1" } }]);
  void useSweep.getState().start({ p99_ms: 500, error_pct: 2 }, "b1");
  expect(useSweep.getState().sent).toEqual({ p99_ms: 500, error_pct: 2, boot: "b1" });
  useSweep.getState().reset();
  expect(useSweep.getState().sent).toBeNull();
});

test("ladderSize_matches_the_server_ladder", () => {
  expect(ladderSize(8)).toBe(11);
  expect(ladderSize(64)).toBe(17);
  expect(ladderSize(1)).toBe(5);
  expect(ladderSize(0)).toBe(1);
  expect(ladderSize(0.1)).toBe(1);
  expect(ladderSize(NaN)).toBe(1);
});

test("one_failed_poll_does_not_end_a_healthy_sweep", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("running", { sweep_points: [pt(0.25)] }) },
    { status: 500, json: { error: "blip" } },
    { json: rec("running", { sweep_points: [pt(0.25), pt(0.35)] }) },
  ]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0);
  await advance(500);
  expect(useSweep.getState().status).toBe("running");
  expect(useSweep.getState().points).toHaveLength(1);
  await advance(500);
  expect(useSweep.getState().status).toBe("running");
  expect(useSweep.getState().points).toHaveLength(2);
});

test("four_failed_polls_in_a_row_fail_the_sweep", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("running", { sweep_points: [pt(0.25)] }) },
    { status: 500, json: { error: "down" } },
  ]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0);
  await advance(1500); // three failures: still running
  expect(useSweep.getState().status).toBe("running");
  await advance(500);
  const s = useSweep.getState();
  expect(s.status).toBe("failed");
  expect(s.error).toBeTruthy();
  expect(s.points).toHaveLength(1);
});

test("poll_error_counter_resets_after_a_good_poll", async () => {
  const fail = { status: 500, json: { error: "blip" } };
  const ok = { json: rec("running", { sweep_points: [pt(0.25)] }) };
  mockFetch([{ status: 202, json: { id: "s1" } }, fail, fail, fail, ok, fail, fail, fail, fail]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0); // 202, then the first 500
  await advance(500 * 3); // two more 500s, then the good poll
  await advance(500 * 3); // three 500s after it
  expect(useSweep.getState().status).toBe("running");
  await advance(500);
  expect(useSweep.getState().status).toBe("failed");
});

test("a_reset_during_the_start_post_is_dropped", async () => {
  const sent = mockFetch([
    { delayMs: 1000, status: 202, json: { id: "s1" } },
    { json: rec("running", { sweep_points: [pt(0.25)] }) },
  ]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0); // POST in flight
  useSweep.getState().reset();
  await advance(3000);
  expect(sent).toHaveLength(1);
  expect(useSweep.getState().status).toBe("idle");
});

test("a_late_answer_after_reset_is_dropped", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { delayMs: 1000, json: rec("running", { sweep_points: [pt(0.25)] }) },
  ]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0); // getRun in flight
  useSweep.getState().reset();
  await advance(3000);
  const s = useSweep.getState();
  expect(s.status).toBe("idle");
  expect(s.record).toBeNull();
  expect(s.points).toHaveLength(0);
});

test("a_second_start_drops_the_first_ones_late_answer", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { delayMs: 1000, json: rec("running", { sweep_points: [pt(0.25)] }) },
    { status: 202, json: { id: "s2" } },
    { json: rec("running", { id: "s2", sweep_points: [pt(0.25), pt(0.35)] }) },
  ]);
  void useSweep.getState().start({ profile: "peak" });
  await advance(0); // first getRun in flight
  void useSweep.getState().start({ profile: "peak" });
  await advance(0);
  expect(useSweep.getState().points).toHaveLength(2);
  await advance(1200); // the first answer lands now
  expect(useSweep.getState().record?.id).toBe("s2");
  expect(useSweep.getState().points).toHaveLength(2);
});

test("a_failed_report_fetch_keeps_the_points", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: rec("done", { sweep: { points: [pt(0.25), pt(0.35)] } }) },
    { status: 500, json: { error: "no report" } },
  ]);
  await useSweep.getState().start({ profile: "peak" });
  const s = useSweep.getState();
  expect(s.status).toBe("done");
  expect(s.report).toBeNull();
  expect(s.error).toBeTruthy();
  expect(s.points).toHaveLength(2);
});
