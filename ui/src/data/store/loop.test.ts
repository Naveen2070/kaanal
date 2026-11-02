import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { ApiError, type StartRun } from "../api/client";
import type { DraftView, RunRecord } from "../api/types";
import { draftBody, startLoop, type LoopDeps } from "./loop";

const RUN_MS = 300;

function viewOf(rev: number, doc: DraftView["doc"] = { nodes: {}, traffic: { peak: {} } }) {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev,
    boot: "b",
    doc,
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  } satisfies DraftView;
}

/** A fake draft, a fake server that runs one simulation at a time, and the numbers on screen. */
function world(scripted: (() => Promise<StartRun>)[] = []) {
  let view: DraftView | null = viewOf(0);
  const listeners = new Set<() => void>();
  const state = {
    status: "idle" as string,
    latest: null as RunRecord | null,
    dimmed: false,
    lastError: null as string | null,
  };
  const log = { starts: [] as number[], maxInFlight: 0 };
  /** The draft rev of every result that reached the screen, in order. */
  const landed: number[] = [];
  let inFlight = 0;
  let n = 0;
  const runs = new Map<string, { rev: number; at: number; counted: boolean }>();
  const failNext = { value: false };

  const deps: LoopDeps = {
    api: {
      async startRun() {
        const next = scripted.shift();
        if (next) return next();
        const id = `s${++n}`;
        runs.set(id, { rev: view?.rev ?? -1, at: Date.now(), counted: false });
        log.starts.push(view?.rev ?? -1);
        inFlight++;
        log.maxInFlight = Math.max(log.maxInFlight, inFlight);
        return { kind: "started", id };
      },
      async getRun(id) {
        const r = runs.get(id);
        if (!r) throw new Error("unknown run");
        const base = {
          id,
          generation: 1,
          profile: "peak",
          scenario: null,
          source: "draft" as const,
          draft_rev: r.rev,
          started_ms: 0,
          finished_ms: null,
        };
        if (Date.now() - r.at < RUN_MS) return { ...base, state: "running" as const };
        if (!r.counted) {
          r.counted = true;
          inFlight--;
        }
        if (failNext.value) {
          failNext.value = false;
          return { ...base, state: "failed" as const, error: "the engine stopped" };
        }
        return { ...base, state: "done" as const };
      },
    },
    draft: {
      getView: () => view,
      subscribe(fn) {
        listeners.add(fn);
        return () => listeners.delete(fn);
      },
    },
    run: {
      set: (p) => {
        Object.assign(state, p);
        if (p.latest) landed.push(p.latest.draft_rev ?? -1);
      },
      hasLatest: () => state.latest !== null,
    },
    sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
    body: draftBody,
  };
  return {
    deps,
    state,
    log,
    landed,
    failNext,
    edit(doc?: DraftView["doc"]) {
      view = viewOf((view?.rev ?? 0) + 1, doc);
      listeners.forEach((f) => f());
    },
    setView(v: DraftView | null) {
      view = v;
      listeners.forEach((f) => f());
    },
    rev: () => view?.rev ?? -1,
  };
}

const tick = (ms: number) => vi.advanceTimersByTimeAsync(ms);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

test("a_burst_of_edits_makes_one_run", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  for (let i = 0; i < 10; i++) {
    w.edit();
    await tick(100);
  }
  await tick(5000);
  expect(w.log.starts).toEqual([10]);
  expect(w.state).toMatchObject({ status: "done", dimmed: false, lastError: null });
  expect(w.state.latest?.draft_rev).toBe(10);
  loop.stop();
});

test("an_edit_during_a_run_triggers_one_more_run_and_the_older_result_is_discarded", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.edit();
  await tick(700); // debounce passed, the run is in flight
  expect(w.log.starts).toEqual([1]);
  w.edit();
  await tick(5000);
  expect(w.log.starts).toEqual([1, 2]);
  // only the run of the newest draft is ever on screen: the older result never landed
  expect(w.landed).toEqual([2]);
  expect(w.state.latest?.draft_rev).toBe(2);
  expect(w.log.maxInFlight).toBe(1);
  loop.stop();
});

test("a_409_waits_and_retries_without_an_error", async () => {
  const busy = async (): Promise<StartRun> => ({ kind: "busy", status: 409, message: "busy" });
  const w = world([busy, busy]);
  const loop = startLoop(w.deps);
  w.edit();
  await tick(700);
  expect(w.state.lastError).toBeNull();
  expect(w.log.starts).toEqual([]);
  await tick(5000);
  expect(w.log.starts).toEqual([1]);
  expect(w.state).toMatchObject({ status: "done", lastError: null });
  loop.stop();
});

test("a_429_backs_off_for_a_second", async () => {
  const w = world([async () => ({ kind: "busy", status: 429, message: "too many" })]);
  const loop = startLoop(w.deps);
  w.edit();
  await tick(700);
  await tick(500);
  expect(w.log.starts).toEqual([]);
  await tick(600);
  expect(w.log.starts).toEqual([1]);
  loop.stop();
});

test("a_400_diagnostic_dims_the_last_numbers_and_sets_blocked", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.edit();
  await tick(5000);
  const good = w.state.latest;
  expect(good).not.toBeNull();
  const refuse = async (): Promise<StartRun> => {
    throw new ApiError(400, "no entry node", {
      file: null,
      line: 1,
      col: 0,
      path: [],
      message: "no entry node",
    });
  };
  const w2 = world([refuse]);
  w2.state.latest = good;
  const loop2 = startLoop(w2.deps);
  w2.edit();
  await tick(5000);
  expect(w2.state).toMatchObject({ status: "blocked", dimmed: true, lastError: "no entry node" });
  expect(w2.state.latest).toBe(good);
  loop.stop();
  loop2.stop();
});

test("a_failed_run_keeps_the_previous_numbers_dimmed_with_the_message", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.edit();
  await tick(5000);
  const good = w.state.latest;
  w.failNext.value = true;
  w.edit();
  await tick(5000);
  expect(w.state).toMatchObject({
    status: "blocked",
    dimmed: true,
    lastError: "the engine stopped",
  });
  expect(w.state.latest).toBe(good);
  loop.stop();
});

test("no_run_starts_while_doc_is_null_and_one_without_a_profile_says_why", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.setView({ ...viewOf(1), doc: null });
  await tick(5000);
  expect(w.log.starts).toEqual([]);
  expect(w.state.lastError).toMatch(/not a valid config/);
  w.setView(viewOf(2, { nodes: {} }));
  await tick(5000);
  expect(w.log.starts).toEqual([]);
  expect(w.state.lastError).toMatch(/no traffic profile/);
  loop.stop();
});

test("a_change_that_is_not_an_edit_of_the_system_starts_nothing", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.edit();
  await tick(5000);
  expect(w.log.starts).toHaveLength(1);
  // a drag of a node notifies the store but leaves rev alone
  w.setView({ ...viewOf(w.rev()), layout: { a: [1, 2] } });
  await tick(5000);
  expect(w.log.starts).toHaveLength(1);
  loop.stop();
});

test("the_stop_function_cancels_polling_and_pending_debounce", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  w.edit();
  await tick(300); // still inside the debounce
  loop.stop();
  await tick(5000);
  expect(w.log.starts).toEqual([]);
  // and a run already polling is dropped: nothing lands on screen
  const w2 = world();
  const loop2 = startLoop(w2.deps);
  w2.edit();
  await tick(700);
  expect(w2.log.starts).toHaveLength(1);
  loop2.stop();
  await tick(5000);
  expect(w2.state.latest).toBeNull();
});

test("run_now_skips_the_wait", async () => {
  const w = world();
  const loop = startLoop(w.deps);
  loop.runNow();
  await tick(10);
  expect(w.log.starts).toEqual([0]);
  loop.stop();
});

// a small seeded generator, so a failure is the same failure next time
function lcg(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}

test("only_one_run_is_ever_in_flight_and_the_last_edit_wins", async () => {
  for (const seed of [1, 2, 3, 42, 2026]) {
    const rand = lcg(seed);
    const busy = async (): Promise<StartRun> => ({ kind: "busy", status: 409, message: "busy" });
    // some starts answer busy before they are accepted
    const w = world(Array.from({ length: Math.floor(rand() * 4) }, () => busy));
    const loop = startLoop(w.deps);
    for (let i = 0; i < 60; i++) {
      if (rand() < 0.7) w.edit();
      await tick(Math.floor(rand() * 900));
    }
    await tick(20_000);
    expect(w.log.maxInFlight, `seed ${seed}`).toBeLessThanOrEqual(1);
    expect(w.state.latest?.draft_rev, `seed ${seed}`).toBe(w.rev());
    expect(w.state.status, `seed ${seed}`).toBe("done");
    // a result for an older draft than one already shown never lands
    expect(w.landed, `seed ${seed}`).toEqual([...w.landed].sort((a, b) => a - b));
    expect(w.landed.at(-1), `seed ${seed}`).toBe(w.rev());
    loop.stop();
  }
});
