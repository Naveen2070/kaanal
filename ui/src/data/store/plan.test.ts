import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { mockFetch } from "../../test/fetch";
import { planDone, planFailed, planRunning } from "../../test/planRecord";
import { TOKEN_KEY } from "../../token";
import { DEFAULT_FORM } from "../../panels/plan/planMath";
import { usePlan } from "./plan";

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  vi.useFakeTimers();
});
afterEach(() => {
  usePlan.getState().reset();
  usePlan.getState().setOpen(false);
  usePlan.getState().setForm(DEFAULT_FORM);
  vi.useRealTimers();
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

const body = { target: { multiple: 3 } };
const advance = (ms: number) => vi.advanceTimersByTimeAsync(ms);

test("start posts, polls while running and keeps the steps as they come", async () => {
  const sent = mockFetch([
    { status: 202, json: { id: "s4" } },
    { json: planRunning() },
    { json: planRunning() },
    { json: planDone() },
    { text: "# Sizing plan s4\n" },
  ]);
  await usePlan.getState().start(body, "boot-1");
  expect(usePlan.getState().status).toBe("running");
  expect(usePlan.getState().record?.plan_steps).toHaveLength(1);
  expect(usePlan.getState().sent).toEqual({ ...body, boot: "boot-1" });
  await advance(500);
  expect(usePlan.getState().status).toBe("running");
  await advance(500);
  expect(usePlan.getState().status).toBe("done");
  expect(usePlan.getState().record?.plan?.verdict.kind).toBe("holds");
  expect(sent[0]?.url).toContain("/sim/plan");
  expect(sent.filter((s) => s.url.includes("/report"))).toHaveLength(1);
  expect(usePlan.getState().report).toBe("# Sizing plan s4\n");
});

test("a report that cannot be fetched keeps the plan done and says why", async () => {
  mockFetch([
    { status: 202, json: { id: "s4" } },
    { json: planDone() },
    { status: 404, json: { error: "unknown run" } },
  ]);
  await usePlan.getState().start(body);
  await advance(0);
  expect(usePlan.getState().status).toBe("done");
  expect(usePlan.getState().report).toBeNull();
  expect(usePlan.getState().error).toContain("unknown run");
});

test("a busy answer is status busy with the server's message", async () => {
  mockFetch([{ status: 409, json: { error: "a run is in progress", id: "s1" } }]);
  await usePlan.getState().start(body);
  expect(usePlan.getState().status).toBe("busy");
  expect(usePlan.getState().error).toContain("a run is in progress");
});

test("a failed record keeps its error", async () => {
  mockFetch([{ status: 202, json: { id: "s4" } }, { json: planFailed() }]);
  await usePlan.getState().start(body);
  expect(usePlan.getState().status).toBe("failed");
  expect(usePlan.getState().error).toBe("the plan failed: no profile");
});

test("three failed polls fail it, one does not", async () => {
  mockFetch([
    { status: 202, json: { id: "s4" } },
    { status: 500, json: { error: "x" } },
    { json: planRunning() },
    { status: 500, json: { error: "x" } },
    { status: 500, json: { error: "x" } },
    { status: 500, json: { error: "x" } },
    { status: 500, json: { error: "x" } },
  ]);
  await usePlan.getState().start(body);
  // the first poll failed once and tries again
  expect(usePlan.getState().status).toBe("running");
  await advance(500);
  expect(usePlan.getState().status).toBe("running");
  await advance(500 * 4);
  expect(usePlan.getState().status).toBe("failed");
});

test("reset stops the timer and drops a late answer", async () => {
  const sent = mockFetch([
    { status: 202, json: { id: "s4" } },
    { json: planRunning() },
    { json: planDone(), delayMs: 100 },
  ]);
  await usePlan.getState().start(body);
  await advance(500);
  usePlan.getState().reset();
  await advance(5000);
  expect(usePlan.getState().status).toBe("idle");
  expect(usePlan.getState().record).toBeNull();
  expect(sent.length).toBeLessThanOrEqual(3);
});

test("closing the dialog does not stop a running poll", async () => {
  mockFetch([
    { status: 202, json: { id: "s4" } },
    { json: planRunning() },
    { json: planDone() },
    { text: "# r" },
  ]);
  usePlan.getState().setOpen(true);
  await usePlan.getState().start(body);
  usePlan.getState().setOpen(false);
  await advance(500);
  expect(usePlan.getState().open).toBe(false);
  expect(usePlan.getState().status).toBe("done");
});

test("a start while a plan runs is ignored, so its answer is not lost", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s4" } }, { json: planRunning() }]);
  await usePlan.getState().start(body);
  const before = sent.length;
  await usePlan.getState().start(body);
  expect(sent).toHaveLength(before);
  expect(usePlan.getState().record?.id).toBe("s4");
  expect(usePlan.getState().status).toBe("running");
});

test("after a reset a new start drops the poll of the old one", async () => {
  mockFetch([
    { status: 202, json: { id: "s4" } },
    { json: planRunning() },
    { status: 202, json: { id: "s5" } },
    { json: { ...planRunning(), id: "s5" } },
  ]);
  await usePlan.getState().start(body);
  usePlan.getState().reset();
  await usePlan.getState().start(body);
  expect(usePlan.getState().record?.id).toBe("s5");
});

test("the form outlives a closed dialog and a reset", () => {
  expect(usePlan.getState().form).toEqual(DEFAULT_FORM);
  const typed = { ...DEFAULT_FORM, target: "5", cheapest: true };
  usePlan.getState().setForm(typed);
  usePlan.getState().setOpen(false);
  usePlan.getState().reset();
  expect(usePlan.getState().form).toEqual(typed);
});
