import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { TOKEN_KEY } from "../token";
import { mockFetch } from "../test/fetch";
import { ApiError, api } from "./client";

beforeEach(() => sessionStorage.setItem(TOKEN_KEY, "tok123"));
afterEach(() => {
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

test("every_call_sends_the_token_header", async () => {
  const sent = mockFetch([{ json: {} }]);
  await api.getDraft();
  await api.putDraft({ layout: {} });
  await api.validate();
  await api.apply();
  await api.save();
  await api.getTopology();
  await api.getRun("s1");
  await api.getLog("s1", { node: "db" });
  await api.sample("GET /x");
  expect(sent).toHaveLength(9);
  for (const s of sent) expect(s.headers.Authorization).toBe("Bearer tok123");
  expect(sent[7]?.url).toBe("/__kaanal/sim/runs/s1/log?node=db");
});

test("a_400_with_a_diagnostic_becomes_an_ApiError_with_the_diagnostic", async () => {
  const diagnostic = { file: null, line: 3, col: 1, path: ["nodes"], message: "bad" };
  mockFetch([{ status: 400, json: { error: "the draft does not build", diagnostic } }]);
  const err = await api.startRun({ profile: "peak", source: "draft" }).catch((e: unknown) => e);
  expect(err).toBeInstanceOf(ApiError);
  expect((err as ApiError).status).toBe(400);
  expect((err as ApiError).message).toBe("the draft does not build");
  expect((err as ApiError).diagnostic).toEqual(diagnostic);
});

test("a_409_run_busy_is_not_thrown_as_a_failure", async () => {
  mockFetch([{ status: 409, json: { error: "a run is in progress", id: "s2" } }]);
  expect(await api.startRun({ profile: "peak" })).toEqual({
    kind: "busy",
    status: 409,
    message: "a run is in progress",
  });
  mockFetch([{ status: 429, json: { error: "too many builds" } }]);
  expect((await api.startRun({ profile: "peak" })).kind).toBe("busy");
  mockFetch([{ status: 202, json: { id: "s3", state: "running" } }]);
  expect(await api.startRun({ profile: "peak" })).toEqual({ kind: "started", id: "s3" });
});

test("other_errors_throw_and_a_cancelled_call_is_aborted", async () => {
  mockFetch([{ status: 500, json: { error: "boom" } }]);
  await expect(api.getDraft()).rejects.toMatchObject({ status: 500, message: "boom" });
  const c = new AbortController();
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_u: string, init: RequestInit) => {
      expect(init.signal).toBe(c.signal);
      throw new DOMException("aborted", "AbortError");
    }),
  );
  await expect(api.getDraft({ signal: c.signal })).rejects.toThrow("aborted");
});
