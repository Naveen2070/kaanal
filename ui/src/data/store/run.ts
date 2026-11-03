// The numbers on screen: the newest finished run and how it relates to the draft. The loop
// (loop.ts) is the only writer; cards and the top bar read it.

import { create } from "zustand";
import type { RunRecord } from "../api/types";

export type RunStatus = "idle" | "running" | "done" | "blocked";

export interface RunState {
  status: RunStatus;
  /** The newest finished run; kept while the next one runs. */
  latest: RunRecord | null;
  /** True while `latest` is older than what the user sees (a run is in flight, or the last try was blocked). */
  dimmed: boolean;
  /** Why the last attempt could not give numbers; shown with the dimmed ones. */
  lastError: string | null;
}

export const useRun = create<RunState>(() => ({
  status: "idle",
  latest: null,
  dimmed: false,
  lastError: null,
}));

/**
 * The finished run before the newest one, kept so Results can say how an edit changed the numbers.
 * Written by watching `latest`; the loop never knows about it.
 */
export const usePrevRun = create<{ prev: RunRecord | null }>(() => ({ prev: null }));

useRun.subscribe((s, before) => {
  if (s.latest && before.latest && s.latest.id !== before.latest.id && before.latest.summary) {
    usePrevRun.setState({ prev: before.latest });
  }
});
