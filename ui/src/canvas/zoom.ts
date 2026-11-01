// The canvas's current zoom, for the zoom pill outside it. The canvas writes it as the view moves;
// the pill only reads it.

import { create } from "zustand";

export const useZoom = create<{ zoom: number }>(() => ({ zoom: 1 }));

/** 0.874 reads "87%". */
export const zoomPercent = (z: number): string =>
  `${Math.round((Number.isFinite(z) ? z : 1) * 100)}%`;
