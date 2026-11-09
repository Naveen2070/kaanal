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
