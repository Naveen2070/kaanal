// Request dots (step 8L, RULES 105 i): a sample of the finished requests the server sends with each
// tick, replayed as calm dots that move along the wires. A dot's speed along a wire is the wire's
// latency (a slow wire makes it crawl); a request that was dropped flashes a mark at its node
// instead (a glyph, never colour alone). At most `MAX_DOTS` are on screen at once.

import type { Journey } from "../../data/api/liveTypes";

/** Most dots on screen at once, over all wires. */
export const MAX_DOTS = 60;
/** The shortest and longest a dot takes along a wire, so a fast wire is still visible and a hung one finite. */
const MIN_TRAVEL_MS = 160;
const MAX_TRAVEL_MS = 4000;
/** How long a dot takes to enter a node's wire when the wire has no latency hop of its own. */
const ENTER_MS = 300;
/** A journey's hops later than this after it arrives are not replayed. */
const HORIZON_MS = 5000;

export interface Dot {
  /** Unique: the tick, the journey, the hop. */
  key: string;
  /** The wire (index into the file's `edges`). */
  edge: number;
  /** Milliseconds after the dot was created before it starts to move. */
  delayMs: number;
  /** How long it takes along the wire. */
  durMs: number;
}

const ms = (ns: number) => ns / 1e6;

/** The dots of the journeys of one tick (`tick` is its `t_ms`, for the keys). */
export function planDots(journeys: readonly Journey[], tick: number): Dot[] {
  const out: Dot[] = [];
  journeys.forEach((j, ji) => {
    j.hops.forEach((h, hi) => {
      if (h.edge === null) return;
      const wire = h.kind === "net" || h.kind === "link";
      const prev = j.hops[hi - 1];
      // an own hop entered over a wire that already has its own latency hop is that same crossing
      if (!wire && prev && prev.edge === h.edge && (prev.kind === "net" || prev.kind === "link")) {
        return;
      }
      const start = ms(h.start_ns);
      const dur = wire
        ? Math.min(MAX_TRAVEL_MS, Math.max(MIN_TRAVEL_MS, ms(h.end_ns - h.start_ns)))
        : ENTER_MS;
      const delay = wire ? start : Math.max(0, start - ENTER_MS);
      if (delay > HORIZON_MS) return;
      out.push({
        key: `${tick}-${ji}-${hi}`,
        edge: h.edge,
        delayMs: Math.round(delay),
        durMs: Math.round(dur),
      });
    });
  });
  return out;
}

/** Adds `fresh` to the dots on screen, dropping the oldest to stay within [`MAX_DOTS`]. */
export function withCap(current: readonly Dot[], fresh: readonly Dot[]): Dot[] {
  const all = [...current, ...fresh];
  return all.length > MAX_DOTS ? all.slice(all.length - MAX_DOTS) : all;
}

/** Whether a dot has finished moving `ageMs` after it was created. */
export const dotDone = (d: Dot, ageMs: number): boolean => ageMs > d.delayMs + d.durMs + 100;

/** Waiting requests drawn as a pile in front of a node: at most 12 dots for any queue. */
export const pileSize = (queue: number): number => Math.min(12, Math.max(0, Math.floor(queue)));
