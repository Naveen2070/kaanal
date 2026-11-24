// Live, batch or admin (step 8L, RULES 105 q; step 8A, RULES 108): the live simulation is the default view; the batch run
// stays for seeded, repeatable comparisons. Not remembered between page loads: Studio opens live.

import { create } from "zustand";

export type Mode = "live" | "batch" | "admin";

/** Live and Admin share the stream, the canvas, the strip and Results; only Batch is a finished run. */
export const isLive = (m: Mode): boolean => m !== "batch";

export const useMode = create<{ mode: Mode; set: (m: Mode) => void }>((set) => ({
  mode: "live",
  set: (mode) => set({ mode }),
}));

/** The slice of `window` `applyStartMode` touches; a test passes a fake. */
export interface ModeEnv {
  location: { pathname: string; search: string };
  history: { replaceState(state: unknown, title: string, url: string): void };
}

/**
 * `?mode=live|batch|admin` on the Studio address picks the mode once, at load (`kaanal up` prints a
 * link that opens a loaded project on Admin). The parameter is then taken off the address bar, so a
 * reload opens as usual (Studio does not remember a mode). Anything else is ignored. Returns the mode set.
 */
export function applyStartMode(env: ModeEnv): Mode | null {
  const params = new URLSearchParams(env.location.search);
  const asked = params.get("mode");
  if (asked === null) return null;
  params.delete("mode");
  const rest = params.toString();
  env.history.replaceState(null, "", env.location.pathname + (rest ? `?${rest}` : ""));
  if (asked !== "live" && asked !== "batch" && asked !== "admin") return null;
  useMode.getState().set(asked);
  return asked;
}
