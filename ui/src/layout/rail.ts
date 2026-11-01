// Whether the details rail is open. It keeps one width on every tab and stays in the row, so the
// canvas never jumps and nothing is covered; a person who wants the room folds it away, and the
// choice is remembered. With no choice yet it starts open on a wide window and folded on a narrow one.

import { create } from "zustand";

const KEY = "kaanal-rail-open";
/** Width of the open rail, in px. */
export const RAIL_W = 360;
/** Width of the folded rail's handle, in px. */
export const RAIL_FOLDED_W = 44;

function initial(): boolean {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "1") return true;
    if (v === "0") return false;
  } catch {
    // blocked storage: the default below
  }
  return typeof window === "undefined" || window.innerWidth >= 1280;
}

export const useRail = create<{ open: boolean; set: (open: boolean) => void }>((set) => ({
  open: initial(),
  set: (open) => {
    try {
      localStorage.setItem(KEY, open ? "1" : "0");
    } catch {
      // blocked storage: this page keeps the choice
    }
    set({ open });
  },
}));
