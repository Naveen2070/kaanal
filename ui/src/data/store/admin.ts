// Admin mode (step 8A, RULES 108): the request feed of a real frontend and the "real only" filter.
// One writer: the poller in `AdminFeed` feeds `addRows`; the strip, the feed and the canvas read it.

import { create } from "zustand";
import type { FeedRow } from "../api/logFeed";

/** The most rows kept; the oldest go first. */
export const MAX_ROWS = 500;

export interface AdminState {
  /** Finished mock requests, oldest first. */
  rows: FeedRow[];
  /** Show only what a real frontend sent: the strip and the canvas read the real share. */
  realOnly: boolean;
  /** Routes whose response edit is live but not yet written into the draft (step 8A). */
  liveOnly: string[];
  addRows: (rows: FeedRow[]) => void;
  setRealOnly: (on: boolean) => void;
  /** An edit went live: it is not in the file yet. */
  markLive: (route: string) => void;
  /** The edit was staged into the draft, or taken back: it is no longer "live only". */
  unmarkLive: (route: string) => void;
  reset: () => void;
}

export const useAdmin = create<AdminState>((set) => ({
  rows: [],
  realOnly: false,
  liveOnly: [],
  addRows: (fresh) =>
    set((s) => {
      // a feed that restarts reads the newest rows again: each request shows once
      const seen = new Set(s.rows.map((r) => r.id));
      const rows = fresh.filter((r) => !seen.has(r.id));
      if (rows.length === 0) return s;
      const all = [...s.rows, ...rows];
      return { rows: all.length > MAX_ROWS ? all.slice(all.length - MAX_ROWS) : all };
    }),
  setRealOnly: (realOnly) => set({ realOnly }),
  markLive: (route) =>
    set((s) => (s.liveOnly.includes(route) ? s : { liveOnly: [...s.liveOnly, route] })),
  unmarkLive: (route) => set((s) => ({ liveOnly: s.liveOnly.filter((r) => r !== route) })),
  reset: () => set({ rows: [], realOnly: false, liveOnly: [] }),
}));
