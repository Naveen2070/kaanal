// Where the first-run flow is: the welcome card, the tour, or neither. A store, so the header's "?"
// can start the tour from anywhere.

import { create } from "zustand";
import { markSeen, neverAgain, setNeverAgain, shouldShowWelcome } from "./flags";

export type Phase = "idle" | "welcome" | "tour" | "off";

interface FirstRunState {
  phase: Phase;
  projectKey: string;
  /** Decides, once per project, whether the card shows. */
  begin: (projectKey: string, isNew: boolean) => void;
  /** The card is closed: remember the project, then the tour (unless told never again). */
  closeWelcome: () => void;
  /** "Don't show again": no card and no tour, for every project. */
  never: () => void;
  endTour: () => void;
  /** The header's "?": the tour, whatever the flags say. */
  replayTour: () => void;
}

export const useFirstRun = create<FirstRunState>((set, get) => ({
  phase: "idle",
  projectKey: "",
  begin(projectKey, isNew) {
    if (get().phase !== "idle" && get().projectKey === projectKey) return;
    // only a project opened as new gets the card: a draft that already has components is real work
    set({ projectKey, phase: isNew && shouldShowWelcome(projectKey) ? "welcome" : "off" });
  },
  closeWelcome() {
    markSeen(get().projectKey);
    set({ phase: neverAgain() ? "off" : "tour" });
  },
  never() {
    setNeverAgain();
    markSeen(get().projectKey);
    set({ phase: "off" });
  },
  endTour: () => set({ phase: "off" }),
  replayTour: () => set({ phase: "tour" }),
}));
