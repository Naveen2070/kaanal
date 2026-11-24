import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { api } from "./client";
import { mockFetch } from "../../test/fetch";
import { TOKEN_KEY } from "../../token";

beforeEach(() => sessionStorage.setItem(TOKEN_KEY, "t"));
afterEach(() => {
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

const body = { target: { multiple: 3 }, p99_ms: 1000 };

test("startPlan posts the body to /sim/plan and returns the id on 202", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s5", state: "running" } }]);
  expect(await api.startPlan(body)).toEqual({ kind: "started", id: "s5" });
  expect(sent[0]?.url).toContain("/sim/plan");
  expect(sent[0]?.method).toBe("POST");
  expect(sent[0]?.body).toEqual(body);
});

test("startPlan returns busy for 409 and 429, and throws for 400", async () => {
  mockFetch([{ status: 409, json: { error: "a run is in progress", id: "s1" } }]);
  expect(await api.startPlan(body)).toMatchObject({ kind: "busy", status: 409 });
  mockFetch([{ status: 429, json: { error: "too many drafts being built at once; retry" } }]);
  expect(await api.startPlan(body)).toMatchObject({ kind: "busy", status: 429 });
  mockFetch([{ status: 400, json: { error: "bad target" } }]);
  await expect(api.startPlan(body)).rejects.toThrow("bad target");
});

test("getPlanReport returns the text and throws on 404", async () => {
  mockFetch([{ text: "# Sizing plan s5\n" }]);
  expect(await api.getPlanReport("s5")).toBe("# Sizing plan s5\n");
  mockFetch([{ status: 404, json: { error: "unknown run" } }]);
  await expect(api.getPlanReport("s9")).rejects.toThrow("unknown run");
});
