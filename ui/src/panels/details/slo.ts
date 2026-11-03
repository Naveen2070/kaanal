// The p99 target a person sets in Results, kept in the browser (per browser, not per project: a
// number to hold the system to while editing). Null is no target. Storage can be blocked: the
// target then lives for this page only.

import { create } from "zustand";

const KEY = "kaanal-slo-p99-ms";

function read(): number | null {
  try {
    const n = Number(localStorage.getItem(KEY));
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

export const useSlo = create<{ targetMs: number | null; set: (ms: number | null) => void }>(
  (set) => ({
    targetMs: read(),
    set: (ms) => {
      try {
        if (ms === null) localStorage.removeItem(KEY);
        else localStorage.setItem(KEY, String(ms));
      } catch {
        // blocked storage: this page keeps it
      }
      set({ targetMs: ms });
    },
  }),
);
