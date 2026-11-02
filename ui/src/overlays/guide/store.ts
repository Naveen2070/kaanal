// Whether the glossary is open and at which chapter. A store, so the book button in the top right,
// the command palette and a help link can all open it. The dialog itself is a lazy chunk.

import { create } from "zustand";

interface GuideState {
  open: boolean;
  chapter: string;
  /** Open the glossary, at `chapter` when given (otherwise where it was left). */
  show: (chapter?: string) => void;
  hide: () => void;
  setChapter: (chapter: string) => void;
}

export const useGuide = create<GuideState>((set) => ({
  open: false,
  chapter: "components",
  show: (chapter) => set((s) => ({ open: true, chapter: chapter ?? s.chapter })),
  hide: () => set({ open: false }),
  setChapter: (chapter) => set({ chapter }),
}));
