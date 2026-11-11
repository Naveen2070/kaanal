// The few helpers every field module uses: a patch op that sets or inserts, the plain-object test and
// the duration spelling. Split out of fields.ts so the machine fields (fields.machine.ts) can share
// them without a cycle.

import type { Op } from "../../data/api/types";

export type Path = (string | number)[];

export const isObject = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === "object" && !Array.isArray(v);

/** `set` when the field is there, `insert` when it is not (a new key under a mapping). */
export function put(path: Path, present: boolean, value: unknown): Op {
  return { op: present ? "set" : "insert", path, value };
}

const DURATION = /^(\d+(?:\.\d+)?)(ms|s)$/;

/** A field written as a plain duration (`98ms`, `2s`) in milliseconds; null for anything else. */
export function durationMs(v: unknown): number | null {
  const m = typeof v === "string" ? DURATION.exec(v) : null;
  return m ? Number(m[1]) * (m[2] === "s" ? 1000 : 1) : null;
}

/** Milliseconds as the loader reads them: plain digits, at most microseconds, never `1e-7ms`. */
export function formatMs(ms: number): string {
  return `${Number(ms.toFixed(3))}ms`;
}
