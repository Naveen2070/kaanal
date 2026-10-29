import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { RunRecord } from "../api/types";
import { mockFetch } from "../test/fetch";
import { useRun } from "./run";

function rec(over: Partial<RunRecord> = {}): RunRecord {
  return {
    id: "s1",
    state: "done",
    generation: 1,
    profile: "peak",
    scenario: null,
    source: "draft",
    draft_rev: 0,
    started_ms: 0,
    finished_ms: 1,
    ...over,
  };
}

beforeEach(() => useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null }));
afterEach(() => vi.unstubAllGlobals());

test("a_run_is_started_polled_and_its_record_kept", async () => {
  const sent = mockFetch([
    { status: 202, json: { id: "s1", state: "running" } },
    { json: rec({ state: "running" }) },
    { json: rec() },
  ]);
  await useRun.getState().request();
  expect(sent[0]?.body).toEqual({ profile: "peak", source: "draft" });
  expect(useRun.getState()).toMatchObject({ status: "done", dimmed: false, lastError: null });
  expect(useRun.getState().latest?.id).toBe("s1");
});

test("a_busy_simulator_keeps_the_last_numbers_dimmed_and_is_not_an_error", async () => {
  mockFetch([{ status: 202, json: { id: "s1" } }, { json: rec() }]);
  await useRun.getState().request();
  mockFetch([{ status: 409, json: { error: "a run is in progress" } }]);
  await useRun.getState().request();
  expect(useRun.getState()).toMatchObject({ status: "blocked", dimmed: true, lastError: null });
  expect(useRun.getState().latest?.id).toBe("s1");
});

test("a_draft_that_does_not_build_keeps_the_last_numbers_and_says_why", async () => {
  mockFetch([{ status: 202, json: { id: "s1" } }, { json: rec() }]);
  await useRun.getState().request();
  mockFetch([{ status: 400, json: { error: "no entry node" } }]);
  await useRun.getState().request();
  expect(useRun.getState()).toMatchObject({
    status: "blocked",
    dimmed: true,
    lastError: "no entry node",
  });
  expect(useRun.getState().latest?.id).toBe("s1");
});

test("an_older_request_drops_its_result_when_a_newer_one_started", async () => {
  mockFetch([{ status: 202, json: { id: "old" }, delayMs: 30 }, { json: rec({ id: "old" }) }]);
  const first = useRun.getState().request();
  mockFetch([{ status: 202, json: { id: "new" } }, { json: rec({ id: "new" }) }]);
  await useRun.getState().request();
  await first;
  expect(useRun.getState().latest?.id).toBe("new");
});
