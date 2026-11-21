// The load sweep on screen: it starts one, polls its record while it runs, keeps the points as
// they come, and fetches the Markdown report once when it is done. No React here; the timer is a
// module-level handle so `reset` can stop it.

import { create } from "zustand";
import { api } from "../api/client";
import type { SweepBody, SweepPointDoc, SweepRecord } from "../api/types";

export type SweepStatus = "idle" | "running" | "done" | "failed" | "busy";

export interface SweepState {
  status: SweepStatus;
  record: SweepRecord | null;
  /** Points so far while running, all of them once done, what ran when it failed. */
  points: SweepPointDoc[];
  /** The Markdown report, fetched once when the sweep is done. */
  report: string | null;
  /** Why it failed or could not start. */
  error: string | null;
  /** The ladder size to expect, for a progress bar: from the request until the record names its own. */
  maxPoints: number;
  /** What the sweep on screen was started with, and the server process (`DraftView.boot`) it went to. */
  sent: (SweepBody & { boot: string | null }) | null;
  start: (body: SweepBody, boot?: string | null) => Promise<void>;
  reset: () => void;
}

const POLL_MS = 500;
const REFINEMENTS = 6;
/** Consecutive failed polls tolerated before the sweep counts as failed. */
const MAX_POLL_ERRORS = 3;
const LOWEST = 0.25;
/** Default `max_factor` of the server. */
const DEFAULT_MAX = 8;

/** Points on the server's ladder: a step of sqrt 2 from 0.25 up to the top (refinements not counted). */
export function ladderSize(maxFactor: number): number {
  if (!Number.isFinite(maxFactor)) return 1;
  return Math.max(1, Math.floor(2 * Math.log2(maxFactor / LOWEST)) + 1);
}

const idle = {
  status: "idle" as SweepStatus,
  record: null,
  points: [] as SweepPointDoc[],
  report: null,
  error: null,
  maxPoints: ladderSize(DEFAULT_MAX) + REFINEMENTS,
  sent: null,
};

/** The ladder size the record itself names. */
const sizeOf = (rec: SweepRecord) => ladderSize(rec.max_factor) + REFINEMENTS;

let timer: ReturnType<typeof setTimeout> | undefined;
/** Bumped by every start and reset; a poll from an older one drops its answer. */
let epoch = 0;

function stop() {
  clearTimeout(timer);
  timer = undefined;
  epoch++;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const useSweep = create<SweepState>((set) => {
  async function poll(id: string, mine: number, errors = 0) {
    try {
      // the same route serves runs and sweeps; the caller started a sweep
      const rec = (await api.getRun(id)) as unknown as SweepRecord;
      if (mine !== epoch) return;
      if (rec.state === "running") {
        set({ record: rec, points: rec.sweep_points, maxPoints: sizeOf(rec) });
        timer = setTimeout(() => void poll(id, mine, 0), POLL_MS);
      } else if (rec.state === "failed") {
        set({
          status: "failed",
          maxPoints: sizeOf(rec),
          record: rec,
          points: rec.sweep_points,
          error: rec.error ?? "the sweep failed",
        });
      } else {
        set({
          record: rec,
          points: rec.sweep?.points ?? rec.sweep_points,
          maxPoints: sizeOf(rec),
        });
        let report: string | null = null;
        let error: string | null = null;
        try {
          report = await api.getSweepReport(id);
        } catch (e) {
          error = message(e);
        }
        if (mine === epoch) set({ status: "done", report, error });
      }
    } catch (e) {
      if (mine !== epoch) return;
      if (errors < MAX_POLL_ERRORS) {
        // a blip must not end a healthy sweep; keep the points and try again
        timer = setTimeout(() => void poll(id, mine, errors + 1), POLL_MS);
      } else set({ status: "failed", error: message(e) });
    }
  }

  return {
    ...idle,
    // Resolves after the first poll; later polls run on the timer.
    async start(body, boot = null) {
      stop();
      const mine = epoch;
      const top = body.max_factor ?? DEFAULT_MAX;
      set({
        ...idle,
        status: "running",
        maxPoints: ladderSize(top) + REFINEMENTS,
        sent: { ...body, boot },
      });
      try {
        const r = await api.startSweep(body);
        if (mine !== epoch) return;
        if (r.kind === "busy") set({ status: "busy", error: r.message });
        else await poll(r.id, mine);
      } catch (e) {
        if (mine === epoch) set({ status: "failed", error: message(e) });
      }
    },
    reset() {
      stop();
      set({ ...idle });
    },
  };
});
