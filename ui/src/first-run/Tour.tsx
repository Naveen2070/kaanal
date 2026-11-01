// The tour (spec 5.3): four steps, Add, Connect, Break, Save. Each one waits for the real action in
// the draft or the run, then moves on by itself. Skip is always there, Escape ends it, and the note
// sits beside its target, never over it. Focus moves into the note and goes back to Run at the end.
// The targets are found by `data-tour`, so the tour does not depend on how those parts are built.

import { useEffect, useRef, useState } from "react";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { useFirstRun } from "./store";

type Side = "right" | "left" | "below";
interface Progress {
  nodes: number;
  edges: number;
  saturated: boolean;
  saved: boolean;
}

const STEPS: {
  id: string;
  target: string;
  side: Side;
  text: string;
  waiting: string;
  done: (now: Progress, start: Progress) => boolean;
}[] = [
  {
    id: "add",
    target: "palette",
    side: "right",
    text: "Add a component: click one in this list, or drag it onto the canvas.",
    waiting: "Waiting for a new component.",
    done: (n, s) => n.nodes > s.nodes,
  },
  {
    id: "connect",
    target: "panel",
    side: "left",
    text: "Connect two components so requests flow between them: drag from one card to another, or pick Connect to… in the details panel.",
    waiting: "Waiting for a connection.",
    done: (n, s) => n.edges > s.edges,
  },
  {
    id: "break",
    target: "load",
    side: "below",
    text: "Break it: raise the offered load until a component turns red, or lower its slots.",
    waiting: "Waiting for something to saturate.",
    done: (n, s) => n.saturated && !s.saturated,
  },
  {
    id: "save",
    target: "save",
    side: "below",
    text: "Happy with it? Save keeps the draft in your project file.",
    waiting: "Waiting for Save.",
    done: (n, s) => n.saved && !s.saved,
  },
];

const WIDTH = 256;
const HEIGHT = 140;

interface Box {
  top: number;
  left: number;
  right: number;
  bottom: number;
}

/**
 * Where a note goes: beside the target on the side that has room, kept on screen, and moved below the
 * target if the clamp would put it over the target. `view` is the window size.
 */
export function placeNote(
  r: Box | null,
  side: Side,
  view: { width: number; height: number },
): { top: number; left: number } {
  if (!r) return { top: 80, left: 80 };
  const clamp = (v: number, max: number) => Math.max(8, Math.min(v, max));
  const maxTop = view.height - HEIGHT;
  const maxLeft = view.width - WIDTH - 8;
  const right = { top: clamp(r.top + 8, maxTop), left: r.right + 8 };
  const left = { top: clamp(r.top + 8, maxTop), left: r.left - WIDTH - 8 };
  const below = { top: r.bottom + 8, left: clamp(r.left, maxLeft) };
  const above = { top: r.top - HEIGHT - 8, left: clamp(r.left, maxLeft) };
  const fits = (p: { top: number; left: number }) =>
    p.left >= 8 &&
    p.top >= 8 &&
    p.left <= maxLeft &&
    p.top <= maxTop + 8 &&
    !(p.left < r.right && p.left + WIDTH > r.left && p.top < r.bottom && p.top + HEIGHT > r.top);
  // the wanted side first, then the others; the first that is on screen and clear of the target wins
  const order = {
    right: [right, left, below, above],
    left: [left, right, below, above],
    below: [below, above, right, left],
  }[side];
  // nothing is both on screen and clear (a tiny window): the wanted side, clamped
  return order.find(fits) ?? { ...below, top: clamp(below.top, maxTop) };
}

function spotFor(target: string, side: Side, resizes: number) {
  // `resizes` is not read: it is an input, so the React Compiler re-runs this (it reads the DOM)
  // after a resize instead of reusing the last answer
  void resizes;
  const r = document.querySelector(`[data-tour="${target}"]`)?.getBoundingClientRect() ?? null;
  return placeNote(r, side, { width: window.innerWidth, height: window.innerHeight });
}

/** How long a step's action must hold before the tour moves on. */
const SETTLE_MS = 400;

export function Tour() {
  const endTour = useFirstRun((s) => s.endTour);
  const progress: Progress = {
    nodes: useDraft((s) => Object.keys(s.view?.doc?.nodes ?? {}).length),
    edges: useDraft((s) => s.view?.doc?.edges?.length ?? 0),
    saturated: useRun((s) => s.latest?.summary?.nodes.some((n) => n.saturated) ?? false),
    saved: useDraft((s) => s.view?.saved ?? false),
  };
  const [i, setI] = useState(0);
  // what each step began with: a step is done by something new, not by what was already there
  const start = useRef<Progress>(progress);
  // bumped on resize so the note follows its target
  const [resizes, setSize] = useState(0);
  const skip = useRef<HTMLButtonElement>(null);
  const step = STEPS[i] as (typeof STEPS)[number];
  const spot = spotFor(step.target, step.side, resizes);

  // the real action moves the tour on; after the last step it ends
  const finished = step.done(progress, start.current);
  // after a beat, so an edit the draft takes straight back out (a refused wire) does not count
  useEffect(() => {
    if (!finished) return;
    const t = setTimeout(() => {
      if (i === STEPS.length - 1) endTour();
      else {
        start.current = progress;
        setI(i + 1);
      }
    }, SETTLE_MS);
    return () => clearTimeout(t);
    // `progress` is rebuilt every render; `finished` is what it decided
    // eslint-disable-next-line react/exhaustive-deps
  }, [finished]);

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

  // focus moves into the note when the tour starts, and then stays with the person's own action (a
  // step that took focus again would pull it away from what they are doing); when the tour ends it
  // goes back to the Run button
  useEffect(() => skip.current?.focus(), []);
  useEffect(() => () => document.querySelector<HTMLElement>('[data-tour="run"]')?.focus(), []);

  return (
    <dialog
      open
      aria-label={`Tour, step ${i + 1} of ${STEPS.length}`}
      style={{ top: spot.top, left: spot.left }}
      className="fixed z-50 m-0 w-64 rounded-lg border-0 bg-tip p-3 text-sm text-on-tip shadow-xl"
    >
      <p className="mb-1">{step.text}</p>
      <p className="mb-2 text-xs opacity-80">{step.waiting}</p>
      <div className="flex items-center gap-2">
        <span className="text-xs opacity-80">
          {i + 1} / {STEPS.length}
        </span>
        <button ref={skip} type="button" className="ml-auto underline" onClick={endTour}>
          Skip tour
        </button>
      </div>
    </dialog>
  );
}
