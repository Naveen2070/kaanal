// Wires the loop to the real API and the real stores. Kept apart from loop.ts so the loop itself
// has no imports it cannot be tested without.

import { api } from "../api/client";
import { useDraft } from "./draft";
import { draftBody, startLoop, type Loop } from "./loop";
import { useRun } from "./run";

let current: Loop | null = null;

/** The Run button: run now, without the wait. */
export function runNow(): void {
  current?.runNow();
}

/** Starts the loop on the real stores; the returned function stops it. */
export function startStudioLoop(): () => void {
  const loop = startLoop({
    api,
    draft: {
      getView: () => useDraft.getState().view,
      subscribe: (fn) => useDraft.subscribe(fn),
    },
    run: {
      set: (patch) => useRun.setState(patch),
      hasLatest: () => useRun.getState().latest !== null,
    },
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    body: draftBody,
  });
  current = loop;
  return () => {
    loop.stop();
    if (current === loop) current = null;
  };
}
