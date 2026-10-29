// A three-stop tour: a short note pinned beside the palette, the details panel and the Run button.
// Next moves on, Skip (or Escape) ends it. The targets are found by `data-tour`, so the tour does
// not depend on how those parts are built. Focus goes to Next so a keyboard user can reach it, and
// back to Run when the tour ends.

import { useEffect, useRef, useState } from "react";
import { useFirstRun } from "./store";

const STOPS = [
  {
    target: "palette",
    side: "right",
    text: "Start here: drag a component onto the canvas, or click it.",
  },
  {
    target: "panel",
    side: "left",
    text: "Select a component to see and change how much it can handle.",
  },
  { target: "run", side: "below", text: "Every change re-runs on its own. Run does it now." },
] as const;

const WIDTH = 256;

interface Spot {
  top: number;
  left: number;
}

/** Beside the target on the side that has room, kept on screen; a missing target gets a fixed corner. */
function spotFor(target: string, side: "right" | "left" | "below", resizes: number): Spot {
  // `resizes` is not read: it is an input, so the React Compiler re-runs this (it reads the DOM)
  // after a resize instead of reusing the last answer
  void resizes;
  const r = document.querySelector(`[data-tour="${target}"]`)?.getBoundingClientRect();
  if (!r) return { top: 80, left: 80 };
  const clamp = (v: number, max: number) => Math.max(8, Math.min(v, max));
  if (side === "right")
    return { top: clamp(r.top + 8, window.innerHeight - 140), left: r.right + 8 };
  if (side === "left") {
    return {
      top: clamp(r.top + 8, window.innerHeight - 140),
      left: clamp(r.left - WIDTH - 8, window.innerWidth - WIDTH),
    };
  }
  return { top: r.bottom + 8, left: clamp(r.left, window.innerWidth - WIDTH - 8) };
}

export function Tour() {
  const endTour = useFirstRun((s) => s.endTour);
  const [i, setI] = useState(0);
  // bumped on resize so the note follows its target
  const [resizes, setSize] = useState(0);
  const next = useRef<HTMLButtonElement>(null);
  const stop = STOPS[i] as (typeof STOPS)[number];
  const last = i === STOPS.length - 1;
  const spot = spotFor(stop.target, stop.side, resizes);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // a card that used this Escape to close has already claimed it
      if (e.key === "Escape" && !e.defaultPrevented) endTour();
    };
    const onResize = () => setSize((n) => n + 1);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
    };
  }, [endTour]);

  // the note takes focus at each stop; when the tour ends, focus goes back to the Run button
  useEffect(() => next.current?.focus(), [i]);
  useEffect(() => () => document.querySelector<HTMLElement>('[data-tour="run"]')?.focus(), []);

  return (
    <dialog
      open
      aria-label={`Tour, step ${i + 1} of ${STOPS.length}`}
      style={{ top: spot.top, left: spot.left }}
      className="fixed z-50 m-0 w-64 rounded-lg border-0 bg-neutral-900 p-3 text-sm text-white shadow-xl"
    >
      <p className="mb-2">{stop.text}</p>
      <div className="flex items-center gap-2">
        <span className="text-xs text-neutral-400">
          {i + 1} / {STOPS.length}
        </span>
        <button type="button" className="ml-auto underline" onClick={endTour}>
          Skip tour
        </button>
        <button
          ref={next}
          type="button"
          className="rounded bg-white px-2 py-0.5 text-neutral-900"
          onClick={() => (last ? endTour() : setI(i + 1))}
        >
          {last ? "Done" : "Next"}
        </button>
      </div>
    </dialog>
  );
}
