// Quick setups for a wire: a named patch of a few keys, so a new user starts from "Lossy link" or
// "Circuit breaker" instead of an empty form, and a power user gets the standard values in one click.
// Basic setups change how the link behaves; advanced ones switch on a failure mechanism. Every value
// is one the docs or the loader give (https://kaanal.naveenr.in/docs/reference/nodes/, RULES 68, 69, 86, 90); the one estimate is
// the slow link's 1 MB/s, which the label says. A setup writes only its own keys (Typical clears
// them all), and the chip shows as pressed when the wire matches it exactly.

import type { EdgeDoc, Op } from "../../../data/api/types";

export interface Preset {
  id: string;
  label: string;
  level: "basic" | "advanced";
  /** One line under the chip's tooltip: what it writes, and where the number comes from. */
  note: string;
  /** The keys it writes, with their values (as the file holds them). */
  set: Record<string, unknown>;
}

/** Every key a setup can write: Typical removes all of them, and "matching" is judged over all of them. */
export const SETUP_KEYS = [
  "bandwidth",
  "loss",
  "tls",
  "keepalive",
  "compress",
  "stream",
  "retry",
  "abandon",
  "breaker",
] as const;

export const PRESETS: readonly Preset[] = [
  {
    id: "typical",
    label: "Typical",
    level: "basic",
    note: "Clears the wire's bandwidth, loss, connection, compression, retry and breaker settings back to the defaults.",
    set: {},
  },
  {
    id: "slow-link",
    label: "Slow link",
    level: "basic",
    note: "Bandwidth 1 MB/s. An estimate: set your own link's rate.",
    set: { bandwidth: 1024 * 1024 },
  },
  {
    id: "lossy-link",
    label: "Lossy link",
    level: "basic",
    note: "Packet loss 1%: one packet in a hundred is lost (the docs' example).",
    set: { loss: 0.01 },
  },
  {
    id: "retry-budget",
    label: "Retry with a budget",
    level: "advanced",
    note: "Up to 3 tries, with gRPC's retry budget so retries cannot feed a storm.",
    set: { retry: { max_attempts: 3, budget: {} } },
  },
  {
    id: "breaker",
    label: "Circuit breaker",
    level: "advanced",
    note: "Opens at half of the last 50 calls failing, for 10 s (resilience4j's count-based breaker).",
    set: { breaker: { window: 50, min_calls: 50, open_for: "10s" } },
  },
  {
    id: "cold-connections",
    label: "Cold connections",
    level: "advanced",
    note: "TLS 1.3 and a new connection for every call: each call pays TCP and TLS set-up.",
    set: { tls: 1.3, keepalive: 1 },
  },
];

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** The patch that applies a setup to the wire at `index`: Typical removes, the rest set or insert. */
export function presetOps(p: Preset, index: number, e: EdgeDoc): Op[] {
  const path = (key: string) => ["edges", index, key];
  if (p.id === "typical") {
    return SETUP_KEYS.filter((k) => k in e).map((k): Op => ({ op: "remove", path: path(k) }));
  }
  return Object.entries(p.set).map(([key, value]): Op => ({
    op: key in e ? "set" : "insert",
    path: path(key),
    value,
  }));
}

/** The setup the wire matches exactly (its keys equal, no other setup key written), or null. */
export function matchingPreset(e: EdgeDoc): string | null {
  const written = SETUP_KEYS.filter((k) => k in e);
  for (const p of PRESETS) {
    const keys = Object.keys(p.set);
    if (keys.length !== written.length) continue;
    if (keys.every((k) => k in e && same((e as Record<string, unknown>)[k], p.set[k]))) return p.id;
  }
  return null;
}
