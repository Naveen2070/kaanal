// The live run on screen (step 8L): the ticks and markers of the session inside the chosen window,
// the state of the generator, and what the last draft edit did to the running system. One writer:
// `liveRuntime.ts` feeds `ingest`; the strip, the timeline and the canvas read it.

import { create } from "zustand";
import type { LiveEdit, LiveEvent, LiveHello, Marker, Tick } from "../api/liveTypes";

/** The shortest and longest window the person may choose, in minutes (RULES 105 k). */
export const MIN_WINDOW_MIN = 5;
export const MAX_WINDOW_MIN = 30;
/** Journeys are only drawn for a few seconds: older ticks drop them to keep memory small. */
const JOURNEY_KEEP_MS = 10_000;

export type LiveStatus = "off" | "connecting" | "running" | "paused" | "hidden-paused";

export interface LiveState {
  status: LiveStatus;
  hello: LiveHello | null;
  /** The window, in minutes: 5 by default, at most 30. */
  windowMin: number;
  ticks: Tick[];
  markers: Marker[];
  /** What the last draft edit did to the running system. */
  lastEdit: LiveEdit | null;
  ingest: (ev: LiveEvent) => void;
  setStatus: (s: LiveStatus) => void;
  setWindow: (minutes: number) => void;
  noteEdit: (e: LiveEdit) => void;
  /** Forgets the run (a fresh start); keeps the window the person chose. */
  reset: () => void;
}

const clampWindow = (m: number) =>
  Math.min(MAX_WINDOW_MIN, Math.max(MIN_WINDOW_MIN, Number.isFinite(m) ? Math.round(m) : 5));

/** The newest ticks and markers that fit `windowMin` minutes before the newest tick. */
function trim<T extends { t_ms?: number; at_ms?: number }>(
  items: T[],
  newest: number,
  windowMin: number,
): T[] {
  const from = newest - windowMin * 60_000;
  const at = (x: T) => x.t_ms ?? x.at_ms ?? 0;
  return items.length > 0 && at(items[0] as T) >= from ? items : items.filter((x) => at(x) >= from);
}

export const useLive = create<LiveState>((set, get) => ({
  status: "off",
  hello: null,
  windowMin: MIN_WINDOW_MIN,
  ticks: [],
  markers: [],
  lastEdit: null,
  ingest(ev) {
    const s = get();
    if (ev.type === "hello") {
      const restarted = s.hello !== null && s.hello.session !== ev.hello.session;
      set({
        hello: ev.hello,
        status: ev.hello.paused ? "paused" : "running",
        // a different session is a new run (an Apply restarted it): the old timeline does not belong to it
        ticks: restarted ? [] : s.ticks,
        markers: restarted
          ? [{ at_ms: 0, kind: "structure", text: "Applied: the live run restarted" }]
          : s.markers,
        // the restart is the Apply: nothing waits for it any more
        lastEdit: restarted ? null : s.lastEdit,
      });
    } else if (ev.type === "tick") {
      const newest = ev.tick.t_ms;
      const ticks = [...s.ticks, ev.tick];
      // journeys are only drawn for a few seconds: strip the ticks that just aged out. Older ones
      // were stripped when they crossed, so the first stripped one ends the walk (a tick costs
      // work in proportion to what is new, not to the 7,200 ticks of a 30 minute window).
      for (let i = ticks.length - 2; i >= 0; i--) {
        const t = ticks[i] as Tick;
        if (t.t_ms >= newest - JOURNEY_KEEP_MS) continue;
        if (t.journeys.length === 0) break;
        ticks[i] = { ...t, journeys: [] };
      }
      set({
        ticks: trim(ticks, newest, s.windowMin),
        markers: trim(s.markers, newest, s.windowMin),
        // the generator stays paused while the page is hidden, so its ticks say paused too: that
        // must not turn "paused because hidden" into "paused by you" (it would never resume)
        status:
          s.status === "hidden-paused" && ev.tick.status.paused
            ? s.status
            : ev.tick.status.paused
              ? "paused"
              : s.status === "paused"
                ? "running"
                : s.status,
      });
    } else if (ev.type === "marker") {
      set({ markers: [...s.markers, ev.marker] });
    } else {
      // the page fell behind and missed ticks: keep the timeline, mark the gap
      const at = s.ticks.at(-1)?.t_ms ?? 0;
      set({
        hello: ev.hello,
        markers: [...s.markers, { at_ms: at, kind: "gap", text: "Some updates were missed" }],
      });
    }
  },
  setStatus: (status) => set({ status }),
  setWindow: (m) => set({ windowMin: clampWindow(m) }),
  noteEdit: (lastEdit) => set({ lastEdit }),
  reset: () => set({ ticks: [], markers: [], hello: null, lastEdit: null }),
}));
