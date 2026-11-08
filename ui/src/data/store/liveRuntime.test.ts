import { beforeEach, expect, test, vi } from "vite-plus/test";
import type { OpenOptions } from "../api/liveClient";
import type { LiveStatus } from "./live";
import { createLive, HIDDEN_GRACE_MS, RECONNECT_MS, type LiveDeps } from "./liveRuntime";

/** A controller over fakes: time is a list of timers the test fires by hand. */
function rig() {
  let status: LiveStatus = "off";
  let hidden = false;
  let vis: (() => void) | null = null;
  const opens: OpenOptions[] = [];
  const closed: number[] = [];
  const timers = new Map<number, { fn: () => void; ms: number }>();
  let next = 1;
  const deps: LiveDeps = {
    open: (o) => {
      opens.push(o);
      const n = opens.length;
      return { close: () => void closed.push(n) };
    },
    pause: vi.fn(() => Promise.resolve()),
    resume: vi.fn(() => Promise.resolve()),
    ingest: vi.fn(),
    setStatus: (s) => void (status = s),
    status: () => status,
    reset: vi.fn(),
    setTimer: (fn, ms) => {
      const id = next++;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimer: (t) => void timers.delete(t as number),
    isHidden: () => hidden,
    onVisibility: (fn) => {
      vis = fn;
      return () => void (vis = null);
    },
  };
  return {
    deps,
    opens,
    closed,
    timers,
    status: () => status,
    setHidden(h: boolean) {
      hidden = h;
      vis?.();
    },
    fire(ms: number) {
      for (const [id, t] of timers) {
        if (t.ms === ms) {
          timers.delete(id);
          t.fn();
        }
      }
    },
  };
}

let r: ReturnType<typeof rig>;
beforeEach(() => void (r = rig()));

test("a_page_load_opens_the_stream_fresh_and_resets_the_view", () => {
  createLive(r.deps).start();
  expect(r.opens).toHaveLength(1);
  expect(r.opens[0]?.fresh).toBe(true);
  expect(r.deps.reset).toHaveBeenCalledTimes(1);
  expect(r.status()).toBe("connecting");
});

test("a_dropped_stream_reconnects_without_fresh_so_the_run_goes_on", () => {
  createLive(r.deps).start();
  r.opens[0]?.onEnd();
  expect(r.opens).toHaveLength(1);
  r.fire(RECONNECT_MS);
  expect(r.opens).toHaveLength(2);
  expect(r.opens[1]?.fresh).toBe(false);
  expect(r.deps.reset).toHaveBeenCalledTimes(1);
});

test("stopping_closes_the_stream_and_a_late_end_does_not_reconnect", () => {
  const c = createLive(r.deps);
  c.start();
  c.stop();
  expect(r.closed).toEqual([1]);
  expect(r.status()).toBe("off");
  r.opens[0]?.onEnd();
  expect(r.timers.size).toBe(0);
});

test("a_hidden_page_pauses_after_60_seconds_and_resumes_when_visible", () => {
  createLive(r.deps).start();
  r.deps.setStatus("running");
  r.setHidden(true);
  expect(r.deps.pause).not.toHaveBeenCalled();
  r.fire(HIDDEN_GRACE_MS);
  expect(r.deps.pause).toHaveBeenCalledTimes(1);
  expect(r.status()).toBe("hidden-paused");
  r.setHidden(false);
  expect(r.deps.resume).toHaveBeenCalledTimes(1);
  expect(r.status()).toBe("running");
});

test("coming_back_inside_the_grace_cancels_the_pause", () => {
  createLive(r.deps).start();
  r.deps.setStatus("running");
  r.setHidden(true);
  r.setHidden(false);
  expect(r.timers.size).toBe(0);
  expect(r.deps.pause).not.toHaveBeenCalled();
  expect(r.deps.resume).not.toHaveBeenCalled();
});

test("a_pause_the_person_chose_is_not_resumed_by_coming_back", async () => {
  const c = createLive(r.deps);
  c.start();
  r.deps.setStatus("running");
  await c.pause();
  r.setHidden(true);
  r.fire(HIDDEN_GRACE_MS);
  r.setHidden(false);
  expect(r.deps.resume).not.toHaveBeenCalled();
});

test("pause_and_resume_send_the_commands_and_set_the_status", async () => {
  const c = createLive(r.deps);
  c.start();
  await c.pause();
  expect(r.status()).toBe("paused");
  await c.resume();
  expect(r.status()).toBe("running");
  expect(r.deps.pause).toHaveBeenCalledTimes(1);
  expect(r.deps.resume).toHaveBeenCalledTimes(1);
});

test("studio_starts_the_live_run_only_when_the_draft_is_a_valid_config_and_opens_it_fresh", async () => {
  const { startStudioLive } = await import("./liveRuntime");
  const { useDraft } = await import("./draft");
  const urls: string[] = [];
  const closed: string[] = [];
  class FakeSource {
    onerror: (() => void) | null = null;
    constructor(public url: string) {
      urls.push(url);
    }
    addEventListener() {}
    close() {
      closed.push(this.url);
    }
  }
  vi.stubGlobal("EventSource", FakeSource);
  useDraft.setState({ view: null });
  const stop = startStudioLive();
  expect(urls).toEqual([]); // a half-typed file waits
  const view = { doc: { nodes: {} }, doc_error: null } as never;
  useDraft.setState({ view });
  expect(urls).toHaveLength(1);
  expect(urls[0]).toContain("fresh=1");
  stop();
  expect(closed).toEqual(urls);
  vi.unstubAllGlobals();
  useDraft.setState({ view: null });
});
