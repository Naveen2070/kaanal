// The dots on screen, shared by every wire (so the cap of 60 holds over all of them). A feeder
// watches the live ticks and adds the dots of each new tick; a dot leaves when it has finished
// moving. Nothing is added while the page is paused or the person prefers reduced motion.

import { useEffect } from "react";
import { create } from "zustand";
import { useLive } from "../../data/store/live";
import { usePrefersReducedMotion } from "../../overlays/first-run/motion";
import { dotDone, planDots, withCap, type Dot } from "./dots";

interface Placed extends Dot {
  /** When it was created (ms, `performance.now()`). */
  born: number;
}

export const useDots = create<{
  dots: Placed[];
  add: (fresh: Dot[], now: number) => void;
  prune: (now: number) => void;
  clear: () => void;
}>((set) => ({
  dots: [],
  add: (fresh, now) =>
    set((s) => ({
      dots: withCap(
        s.dots,
        fresh.map((d) => ({ ...d, born: now })),
      ) as Placed[],
    })),
  prune: (now) => set((s) => ({ dots: s.dots.filter((d) => !dotDone(d, now - d.born)) })),
  clear: () => set({ dots: [] }),
}));

/** Feeds the store from the live ticks; mount it once. */
export function useDotFeeder(): void {
  const still = usePrefersReducedMotion();
  useEffect(() => {
    if (still) {
      useDots.getState().clear();
      return;
    }
    let seen = useLive.getState().ticks.at(-1)?.t_ms ?? -1;
    const off = useLive.subscribe((s) => {
      const t = s.ticks.at(-1);
      if (!t || t.t_ms === seen) return;
      seen = t.t_ms;
      if (s.status !== "running" || t.journeys.length === 0) return;
      useDots.getState().add(planDots(t.journeys, t.t_ms), performance.now());
    });
    const timer = setInterval(() => useDots.getState().prune(performance.now()), 500);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [still]);
}
