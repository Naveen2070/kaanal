// "Measure" in Live: one batch run of the draft, on demand, into the same store Results reads. The
// auto re-run loop is stopped in Live (only Batch runs it), so this is the one way a finished run
// appears there. A busy server (409) or a build limit (429) is waited out a few times, never shown
// as an error; a draft that cannot run is the message.

import { api, ApiError, type StartRun } from "../api/client";
import type { RunBody, RunRecord } from "../api/types";
import { useDraft } from "./draft";
import { draftBody } from "./loop";
import { useRun } from "./run";

const TRIES = 20;

interface Deps {
  startRun(body: RunBody): Promise<StartRun>;
  getRun(id: string): Promise<RunRecord>;
  sleep(ms: number): Promise<void>;
}

/** Resolves with a message when it could not run, null when a run landed in the store. */
export async function measure(
  body: RunBody | null,
  d: Deps = { ...api, sleep: (ms) => new Promise((r) => setTimeout(r, ms)) },
): Promise<string | null> {
  if (!body) return "The draft has no traffic profile to run.";
  const had = useRun.getState().latest !== null;
  useRun.setState({ status: "running", dimmed: had });
  try {
    let id: string | null = null;
    for (let i = 0; i < TRIES && id === null; i++) {
      const r = await d.startRun(body);
      if (r.kind === "started") id = r.id;
      else await d.sleep(r.status === 429 ? 1000 : 500);
    }
    if (id === null) throw new Error("The server stayed busy; try again.");
    for (;;) {
      const rec = await d.getRun(id);
      if (rec.state === "running") {
        await d.sleep(100);
        continue;
      }
      if (rec.state === "failed") throw new Error(rec.error ?? "The run failed.");
      useRun.setState({ status: "done", latest: rec, dimmed: false, lastError: null });
      return null;
    }
  } catch (e) {
    const msg = e instanceof ApiError || e instanceof Error ? e.message : String(e);
    useRun.setState({ status: "blocked", dimmed: had, lastError: msg });
    return msg;
  }
}

/** Fewest and most requests a second a profile may ask for (`traffic:`, config/traffic.rs). */
const RPS_RANGE = [1e-6, 1e6] as const;

/**
 * Measures the draft as it is now; with `rate`, at that many requests a second instead of the
 * profile's own (the live offered rate), the profile's mix kept.
 */
export const measureDraft = (rate?: number): Promise<string | null> => {
  const view = useDraft.getState().view;
  const body = view?.doc ? draftBody(view) : null;
  if (body && typeof body.profile === "string" && rate !== undefined) {
    const own = (view?.doc?.traffic as Record<string, unknown> | undefined)?.[body.profile];
    if (rate >= RPS_RANGE[0] && rate <= RPS_RANGE[1] && own && typeof own === "object") {
      body.profile = { ...own, rps: rate };
    }
  }
  return measure(body);
};
