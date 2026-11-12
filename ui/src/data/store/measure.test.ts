import { beforeEach, expect, test, vi } from "vite-plus/test";
import type { RunRecord } from "../api/types";
import { measure } from "./measure";
import { useRun } from "./run";

const rec = (state: string, extra = {}) => ({ id: "s1", state, ...extra }) as unknown as RunRecord;
const sleep = () => Promise.resolve();
beforeEach(() => useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null }));

test("a_run_lands_in_the_store_the_results_read_and_a_busy_server_is_waited_out", async () => {
  const startRun = vi
    .fn()
    .mockResolvedValueOnce({ kind: "busy", status: 409 })
    .mockResolvedValueOnce({ kind: "started", id: "s1" });
  const getRun = vi.fn().mockResolvedValueOnce(rec("running")).mockResolvedValueOnce(rec("done"));
  expect(await measure({ profile: "p", source: "draft" }, { startRun, getRun, sleep })).toBeNull();
  expect(startRun).toHaveBeenCalledTimes(2);
  expect(useRun.getState().status).toBe("done");
  expect(useRun.getState().latest?.id).toBe("s1");
});

test("a_failed_run_or_no_profile_is_a_message_and_keeps_the_last_numbers", async () => {
  useRun.setState({ latest: rec("done") });
  expect(await measure(null)).toMatch(/no traffic profile/);
  const startRun = vi.fn().mockResolvedValue({ kind: "started", id: "s2" });
  const getRun = vi.fn().mockResolvedValue(rec("failed", { error: "boom" }));
  expect(await measure({ profile: "p", source: "draft" }, { startRun, getRun, sleep })).toBe(
    "boom",
  );
  expect(useRun.getState().latest?.id).toBe("s1");
  expect(useRun.getState().dimmed).toBe(true);
});

test("measureDraft_runs_the_profile_at_the_given_rate_and_keeps_its_mix", async () => {
  const { api } = await import("../api/client");
  const { useDraft } = await import("./draft");
  const { measureDraft } = await import("./measure");
  const journeys = [{ endpoint: "GET /x", weight: 2 }];
  useDraft.setState({
    view: { doc: { traffic: { p: { rps: 100, journeys } } }, rev: 1, boot: "b" } as never,
  });
  const start = vi.spyOn(api, "startRun").mockResolvedValue({ kind: "started", id: "s9" } as never);
  vi.spyOn(api, "getRun").mockResolvedValue(rec("done"));
  await measureDraft(500);
  expect(start).toHaveBeenLastCalledWith({
    profile: { rps: 500, journeys },
    source: "draft",
  });
  // no rate, or one the server would refuse: the profile by name, as before
  await measureDraft();
  expect(start).toHaveBeenLastCalledWith({ profile: "p", source: "draft" });
  await measureDraft(2e6);
  expect(start).toHaveBeenLastCalledWith({ profile: "p", source: "draft" });
  vi.restoreAllMocks();
});
