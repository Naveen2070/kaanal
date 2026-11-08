// Keeps the live run going while Studio is open (step 8L, RULES 105): opens the stream once the
// draft is a valid config, reconnects when it drops (without `fresh`, so the run goes on), pauses
// the generator 60 seconds after the page is hidden and resumes it on return, and sends the
// person's pause, resume and rate. Time, the stream and the API are injected so tests drive it.

import { api } from "../api/client";
import { openLive, type OpenOptions } from "../api/liveClient";
import type { LiveEvent } from "../api/liveTypes";
import { useDraft } from "./draft";
import { useLive, type LiveStatus } from "./live";

/** How long a hidden page keeps the run going before it pauses (RULES 105 m). */
export const HIDDEN_GRACE_MS = 60_000;
/** How long to wait before reconnecting a dropped stream. */
export const RECONNECT_MS = 1000;

export interface LiveDeps {
  open: (o: OpenOptions) => { close: () => void };
  pause: () => Promise<unknown>;
  resume: () => Promise<unknown>;
  ingest: (ev: LiveEvent) => void;
  setStatus: (s: LiveStatus) => void;
  status: () => LiveStatus;
  reset: () => void;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (t: unknown) => void;
  isHidden: () => boolean;
  /** Calls `fn` when the page is hidden or shown; returns the way to stop listening. */
  onVisibility: (fn: () => void) => () => void;
}

export interface LiveController {
  start(): void;
  stop(): void;
  pause(): Promise<void>;
  resume(): Promise<void>;
}

export function createLive(d: LiveDeps): LiveController {
  let handle: { close: () => void } | null = null;
  let wanted = false;
  let retry: unknown = null;
  let hiddenTimer: unknown = null;
  let unlisten: (() => void) | null = null;

  const connect = (fresh: boolean) => {
    d.setStatus("connecting");
    handle = d.open({
      fresh,
      onEvent: d.ingest,
      onEnd: () => {
        handle = null;
        if (!wanted) return;
        // the stream ended (a restart after an Apply, a network blip): the same page goes on, no `fresh`
        retry = d.setTimer(() => connect(false), RECONNECT_MS);
      },
    });
  };

  const onVisibility = () => {
    if (!wanted) return;
    if (d.isHidden()) {
      if (hiddenTimer === null) {
        hiddenTimer = d.setTimer(() => {
          hiddenTimer = null;
          if (d.status() === "running") {
            void d.pause();
            d.setStatus("hidden-paused");
          }
        }, HIDDEN_GRACE_MS);
      }
    } else {
      if (hiddenTimer !== null) {
        d.clearTimer(hiddenTimer);
        hiddenTimer = null;
      }
      // the grace ran out while it was hidden: resume now that it is back
      if (d.status() === "hidden-paused") {
        void d.resume();
        d.setStatus("running");
      }
    }
  };

  return {
    start() {
      if (wanted) return;
      wanted = true;
      d.reset();
      unlisten = d.onVisibility(onVisibility);
      connect(true);
    },
    stop() {
      wanted = false;
      handle?.close();
      handle = null;
      if (retry !== null) d.clearTimer(retry);
      if (hiddenTimer !== null) d.clearTimer(hiddenTimer);
      retry = hiddenTimer = null;
      unlisten?.();
      unlisten = null;
      d.setStatus("off");
    },
    async pause() {
      await d.pause();
      d.setStatus("paused");
    },
    async resume() {
      await d.resume();
      d.setStatus("running");
    },
  };
}

let current: LiveController | null = null;

/** The pause button. */
export const pauseLive = (): Promise<void> => current?.pause() ?? Promise.resolve();
/** The resume button. */
export const resumeLive = (): Promise<void> => current?.resume() ?? Promise.resolve();

/**
 * Starts the live run on the real stores as soon as the draft is a config (a half-typed file
 * waits); the returned function stops it. A draft that turns invalid later does not stop it: the
 * applied system keeps running.
 */
export function startStudioLive(): () => void {
  const c = createLive({
    open: openLive,
    pause: () => api.livePause(),
    resume: () => api.liveResume(),
    ingest: (ev) => useLive.getState().ingest(ev),
    setStatus: (s) => useLive.getState().setStatus(s),
    status: () => useLive.getState().status,
    reset: () => useLive.getState().reset(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (t) => clearTimeout(t as ReturnType<typeof setTimeout>),
    isHidden: () => document.hidden,
    onVisibility: (fn) => {
      document.addEventListener("visibilitychange", fn);
      return () => document.removeEventListener("visibilitychange", fn);
    },
  });
  current = c;
  const ready = () => useDraft.getState().view?.doc != null;
  let off: (() => void) | null = null;
  if (ready()) c.start();
  else {
    off = useDraft.subscribe(() => {
      if (ready()) {
        off?.();
        off = null;
        c.start();
      }
    });
  }
  return () => {
    off?.();
    c.stop();
    if (current === c) current = null;
  };
}
