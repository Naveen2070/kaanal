// Whether the glossary is open and at which chapter. A store, so the book button in the top right,
// the command palette and a help link can all open it. The dialog itself is a lazy chunk, and so
// are its words: this store must not import them (the first-load size budget, spec 11), so an entry
// is only remembered here and the dialog finds its chapter.

import { create } from "zustand";

/** A chapter id, or where to land: an entry (the dialog finds its chapter) and/or a chapter. */
export type GuideTarget = string | { chapter?: string; entry?: string };

interface GuideState {
  open: boolean;
  chapter: string;
  /** The entry a help link asked for: the dialog shows its chapter, scrolls to it and marks it. */
  entry: string | null;
  /** Open the glossary, at `target` when given (otherwise where it was left). */
  show: (target?: GuideTarget) => void;
  hide: () => void;
  /** Choosing a chapter by hand forgets any entry that was asked for. */
  setChapter: (chapter: string) => void;
}

export const useGuide = create<GuideState>((set) => ({
  open: false,
  chapter: "components",
  entry: null,
  show: (target) =>
    set((s) => {
      if (target === undefined) return { open: true, entry: null };
      if (typeof target === "string") return { open: true, chapter: target, entry: null };
      return { open: true, chapter: target.chapter ?? s.chapter, entry: target.entry ?? null };
    }),
  hide: () => set({ open: false }),
  setChapter: (chapter) => set({ chapter, entry: null }),
}));
