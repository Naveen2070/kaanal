// The wire picked on the canvas, by its place in `edges`, for the rail to show its settings. It is
// not in the address bar: a place in the list shifts when a wire goes, so it would not survive a
// reload. Kept apart from the node selection: picking one clears the other (the canvas does that).

import { create } from "zustand";

export const usePickedWire = create<{ index: number | null; set: (index: number | null) => void }>(
  (set) => ({ index: null, set: (index) => set({ index }) }),
);
