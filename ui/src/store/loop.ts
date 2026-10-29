// The auto re-run loop (spec section 8). Every edit of the draft (a new `rev`) starts a short wait;
// when the edits stop, one run of the draft is started, polled to its end, and its numbers replace
// the ones on screen. Rules, each pinned by a test:
//  - exactly one run in flight, ever; an edit during a run makes one more run after it
//  - a result built from an older draft than the newest edit is thrown away
//  - 409 (the server is busy) and 429 (too many builds) are waited out, never shown as errors
//  - a draft that does not build (400), a failed run, or no profile to run keep the last good numbers,
//    dimmed, with the message
//  - nothing runs while the draft is not a config (`doc` is null: half-typed text)
// Time, the API and the stores are injected, so the tests drive it with fake timers.

import type { DraftView, RunBody, RunRecord } from "../api/types";
import { ApiError, type StartRun } from "../api/client";

export interface LoopDeps {
  api: {
    startRun(body: RunBody): Promise<StartRun>;
    getRun(id: string): Promise<RunRecord>;
  };
  draft: {
    getView(): DraftView | null;
    /** Calls `fn` after every change of the store; returns the way to stop listening. */
    subscribe(fn: () => void): () => void;
  };
  run: {
    set(patch: {
      status?: "idle" | "running" | "done" | "blocked";
      latest?: RunRecord | null;
      dimmed?: boolean;
      lastError?: string | null;
    }): void;
    hasLatest(): boolean;
  };
  sleep(ms: number): Promise<void>;
  /** What to simulate for this draft; null when the draft has nothing to run. */
  body(view: DraftView): RunBody | null;
  debounceMs?: number;
  pollMs?: number;
  busyMs?: number;
  backoffMs?: number;
}

export interface Loop {
  stop(): void;
  /** Runs now (the Run button), skipping the wait. */
  runNow(): void;
}

/** The first traffic profile of the draft, as the body of a draft run; null when there is none. */
export function draftBody(view: DraftView): RunBody | null {
  const traffic = view.doc?.traffic;
  if (traffic === null || typeof traffic !== "object" || Array.isArray(traffic)) return null;
  const first = Object.keys(traffic)[0];
  return first === undefined ? null : { profile: first, source: "draft" };
}

export function startLoop(deps: LoopDeps): Loop {
  const debounce = deps.debounceMs ?? 600;
  const pollMs = deps.pollMs ?? 100;
  const busyMs = deps.busyMs ?? 500;
  const backoffMs = deps.backoffMs ?? 1000;
  let stopped = false;
  let working = false;
  /** Another pass is wanted when the current one ends. */
  let again = false;
  /** Bumped by every edit; a wait that sees a newer token has been superseded. */
  let token = 0;
  let lastKey = "";
  /** The draft (boot:rev) the run in flight, or the last one, was started for. */
  let covered = "";

  const rev = () => deps.draft.getView()?.rev ?? -1;
  const keyOf = (v: DraftView | null) => (v ? `${v.boot}:${v.rev}` : "");
  const dim = () => deps.run.hasLatest();

  /** Starts a run, waiting out a busy server; null when stopped or superseded by a newer edit. */
  async function start(body: RunBody): Promise<string | null> {
    for (;;) {
      if (stopped) return null;
      const r = await deps.api.startRun(body);
      if (r.kind === "started") return r.id;
      // busy: not an error, wait and try again (with the draft as it is by then)
      await deps.sleep(r.status === 429 ? backoffMs : busyMs);
      if (again) return null;
    }
  }

  async function pass(): Promise<void> {
    const view = deps.draft.getView();
    if (!view?.doc) {
      deps.run.set({
        status: "blocked",
        dimmed: dim(),
        lastError: "The draft is not a valid config yet.",
      });
      return;
    }
    const body = deps.body(view);
    if (!body) {
      deps.run.set({
        status: "blocked",
        dimmed: dim(),
        lastError: "The draft has no traffic profile to run.",
      });
      return;
    }
    const mine = view.rev;
    covered = keyOf(view);
    deps.run.set({ status: "running", dimmed: dim() });
    try {
      const id = await start(body);
      if (id === null) return;
      for (;;) {
        const rec = await deps.api.getRun(id);
        if (stopped) return;
        if (rec.state === "running") {
          await deps.sleep(pollMs);
          continue;
        }
        if (rec.state === "failed") {
          deps.run.set({
            status: "blocked",
            dimmed: dim(),
            lastError: rec.error ?? "The run failed.",
          });
          return;
        }
        // built from an older draft than the newest edit: its numbers describe a system that is gone
        // (the pause after that edit is still running or has set `again`: no rush here)
        if ((rec.draft_rev ?? mine) < rev()) return;
        deps.run.set({ status: "done", latest: rec, dimmed: false, lastError: null });
        return;
      }
    } catch (e) {
      if (stopped) return;
      // a draft that does not build is a 400 with a message: keep the last numbers, dimmed
      deps.run.set({
        status: "blocked",
        dimmed: dim(),
        lastError: e instanceof ApiError ? e.message : String(e),
      });
    }
  }

  async function work(): Promise<void> {
    if (working) {
      again = true;
      return;
    }
    working = true;
    try {
      do {
        again = false;
        await pass();
      } while (again && !stopped);
    } finally {
      working = false;
    }
  }

  const schedule = () => {
    const mine = ++token;
    void deps.sleep(debounce).then(() => {
      if (stopped || mine !== token) return;
      // a run already started (or finished) for this very draft covers it: no second one
      if (keyOf(deps.draft.getView()) === covered) return;
      if (working) again = true;
      else void work();
    });
  };

  const unsubscribe = deps.draft.subscribe(() => {
    const view = deps.draft.getView();
    // only a change of the system starts a run: not a drag of a node, not a selection
    const key = keyOf(view);
    if (key === lastKey) return;
    lastKey = key;
    if (view) schedule();
  });

  return {
    stop() {
      stopped = true;
      token++;
      unsubscribe();
    },
    runNow() {
      token++;
      if (!stopped) void work();
    },
  };
}
