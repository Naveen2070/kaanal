// Whether the details rail is open. It keeps one width on every tab and stays in the row, so the
// canvas never jumps and nothing is covered; a person who wants the room folds it away, and the
// choice is remembered. With no choice yet it starts open on a wide window and folded on a narrow one.

import { create } from "zustand";

const KEY = "kaanal-rail-open";
/** Width of the open rail, in px. */
export const RAIL_W = 360;
/** The width a person can drag the open rail to, in px. */
export const RAIL_MIN_W = 320;
export const RAIL_MAX_W = 640;
const WIDTH_KEY = "kaanal-rail-w";
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

function initialWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY));
    if (Number.isFinite(n) && n >= RAIL_MIN_W && n <= RAIL_MAX_W) return Math.round(n);
  } catch {
    // blocked storage: the default
  }
  return RAIL_W;
}

export const useRail = create<{
  open: boolean;
  set: (open: boolean) => void;
  /** The open rail's width in px; dragged or moved by keys, remembered. */
  width: number;
  setWidth: (px: number) => void;
}>((set) => ({
  open: initial(),
  width: initialWidth(),
  setWidth: (px) => {
    const width = Math.round(Math.min(RAIL_MAX_W, Math.max(RAIL_MIN_W, px)));
    try {
      localStorage.setItem(WIDTH_KEY, String(width));
    } catch {
      // blocked storage: this page keeps the choice
    }
    set({ width });
  },
  set: (open) => {
    try {
      localStorage.setItem(KEY, open ? "1" : "0");
    } catch {
      // blocked storage: this page keeps the choice
    }
    set({ open });
  },
}));
