// The Live tab as data: the rules a pin must meet (the control API refuses the rest, so the form
// never sends one), the body for a pin, and the short sentences shown for pins and leases. Pure, so
// it is tested as a table without a panel.

import type { LivePin, PinBody, PinState } from "../../data/api/types";

/** The lease choices when a scenario is turned on: none means until turned off. */
export const LEASES = [
  { label: "until turned off", ttl: undefined },
  { label: "for 30 seconds", ttl: "30s" },
  { label: "for 5 minutes", ttl: "5m" },
] as const;

export const DELAY_MAX_MS = 300_000;

/** Whether a state takes an HTTP status of its own, and the range the API allows. */
export function statusRange(state: PinState): { min: number; max: number } | null {
  if (state === "error") return { min: 400, max: 599 };
  if (state === "empty") return { min: 200, max: 299 };
  return null;
}

/** Whether a state takes a delay: every one but `success`, which is the real answer. */
export const takesDelay = (state: PinState): boolean => state !== "success";

/**
 * The body for pinning `endpoint` as `state`, or the one thing wrong in words. `status` and `delay`
 * are the raw text of their boxes; an empty box means the state's own default.
 */
export function pinBody(
  endpoint: string,
  state: PinState,
  status: string,
  delayMs: string,
): { body: PinBody } | { problem: string } {
  if (endpoint === "") return { problem: "Pick an endpoint." };
  const body: PinBody = { endpoint, state };
  const range = statusRange(state);
  if (status.trim() !== "" && range) {
    const n = Number(status);
    if (!Number.isInteger(n) || n < range.min || n > range.max) {
      return { problem: `The status is a whole number from ${range.min} to ${range.max}.` };
    }
    body.status = n;
  }
  if (delayMs.trim() !== "" && takesDelay(state)) {
    const n = Number(delayMs);
    if (!Number.isFinite(n) || n <= 0 || n > DELAY_MAX_MS) {
      return {
        problem: `The delay is above 0 and at most ${DELAY_MAX_MS.toLocaleString("en-US")} ms.`,
      };
    }
    body.delay = `${Number(n.toFixed(3))}ms`;
  }
  return { body };
}

/** `error 429, after 500 ms`: what a pin makes the endpoint do. */
export function describePin(p: Pick<LivePin, "state" | "status" | "delay_ms">): string {
  const status = p.status === undefined ? "" : ` ${p.status}`;
  const delay = p.delay_ms === undefined ? "" : `, after ${p.delay_ms} ms`;
  return `${p.state}${status}${delay}`;
}

/** `29 s left`, rounded up, or `4 min left`. */
export function leaseLeft(ms: number): string {
  const s = Math.ceil(ms / 1000);
  return s < 120 ? `${s} s left` : `${Math.ceil(s / 60)} min left`;
}

/** Where a scenario is defined, in plain words. */
export const SOURCE: Record<"inline" | "file" | "runtime", string> = {
  inline: "in kaanal.yaml",
  file: "in scenarios/",
  runtime: "made here",
};

/**
 * "Apply and start": press the top bar's own Apply (one code path for Apply), wait until the applied
 * system answers, then start a run. `false` when it never came within `tries` looks, `gapMs` apart
 * (Apply refused, or the dialog it opens is still waiting for an answer); nothing runs then.
 */
export async function applyAndStart(deps: {
  press: () => void;
  ready: () => Promise<boolean>;
  run: () => void;
  sleep: (ms: number) => Promise<void>;
  tries?: number;
  gapMs?: number;
}): Promise<boolean> {
  const { press, ready, run, sleep, tries = 40, gapMs = 250 } = deps;
  press();
  for (let i = 0; i < tries; i++) {
    await sleep(gapMs);
    if (await ready().catch(() => false)) {
      run();
      return true;
    }
  }
  return false;
}
