// The simulation run as the UI holds it. This is the plain request/poll cycle; the auto re-run loop
// (debounce, one run in flight, stale results) is built on it in Task C3.

import { create } from "zustand";
import { ApiError, api } from "../api/client";
import type { RunBody, RunRecord } from "../api/types";

export type RunStatus = "idle" | "running" | "done" | "blocked";

interface RunState {
  status: RunStatus;
  /** The newest finished run; kept while the next one runs. */
  latest: RunRecord | null;
  /** True while `latest` is older than what the user sees (a run is in flight, or the last try was blocked). */
  dimmed: boolean;
  lastError: string | null;
  /** Starts a run of `body` (a draft run of the `peak` profile by default) and waits for it. */
  request: (body?: RunBody) => Promise<void>;
}

const POLL_MS = 100;
const DEFAULT_BODY: RunBody = { profile: "peak", source: "draft" };

export const useRun = create<RunState>((set, get) => {
  /** Newer requests make older ones drop their result. */
  let seq = 0;

  return {
    status: "idle",
    latest: null,
    dimmed: false,
    lastError: null,
    async request(body = DEFAULT_BODY) {
      const mine = ++seq;
      set({ status: "running", dimmed: get().latest !== null });
      try {
        const started = await api.startRun(body);
        if (started.kind === "busy") {
          // busy is not a failure: keep what is on screen and let the caller try again
          if (mine === seq)
            set({ status: "blocked", dimmed: get().latest !== null, lastError: null });
          return;
        }
        for (;;) {
          const rec = await api.getRun(started.id);
          if (mine !== seq) return;
          if (rec.state === "running") {
            await new Promise((r) => setTimeout(r, POLL_MS));
            continue;
          }
          if (rec.state === "done") {
            set({ status: "done", latest: rec, dimmed: false, lastError: null });
          } else {
            set({
              status: "blocked",
              dimmed: get().latest !== null,
              lastError: rec.error ?? "the run failed",
            });
          }
          return;
        }
      } catch (e) {
        if (mine !== seq) return;
        // a draft that does not build (400 with a diagnostic) keeps the last good numbers on screen
        set({
          status: "blocked",
          dimmed: get().latest !== null,
          lastError: e instanceof ApiError ? e.message : String(e),
        });
      }
    },
  };
});
