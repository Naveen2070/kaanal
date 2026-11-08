// Live or batch (step 8L, RULES 105 q): the live simulation is the default view; the batch run
// stays for seeded, repeatable comparisons. Not remembered between page loads: Studio opens live.

import { create } from "zustand";

export type Mode = "live" | "batch";

export const useMode = create<{ mode: Mode; set: (m: Mode) => void }>((set) => ({
  mode: "live",
  set: (mode) => set({ mode }),
}));
