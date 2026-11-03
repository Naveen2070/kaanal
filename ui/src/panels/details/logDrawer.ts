// Whether the Log is popped out of the rail into a drawer along the bottom of the window. The rail
// is narrow for a table; the drawer has the width. Not remembered: a reload brings it back.

import { create } from "zustand";

export const useLogDrawer = create<{ out: boolean; set: (out: boolean) => void }>((set) => ({
  out: false,
  set: (out) => set({ out }),
}));
