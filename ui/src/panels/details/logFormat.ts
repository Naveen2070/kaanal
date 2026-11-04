// One line of the run log as text, like the spec's example:
//   t=12.31s WARN r-4812 POST /orders api#2 queued 210ms, served 98ms
// The stamp is VIRTUAL time inside the simulation, and the lines are simulated trace events, not
// process output (the tab says so). `#2` after a node name is display only.

import { fmtMs } from "../../canvas/metrics";
import type { LogLine } from "../../data/api/types";

function ms(n: number): string {
  return `${Number(n.toFixed(n < 10 ? 2 : 1))}ms`;
}

/** Who a line is about: a component (with its instance), or for a hop on a wire, "from → to". */
export function whoOf(l: LogLine): string {
  if (l.from) return `${l.from} → ${l.node}`;
  return l.replica === null ? l.node : `${l.node}#${l.replica}`;
}

export function formatLine(l: LogLine): string {
  const at = `t=${(l.t_ms / 1000).toFixed(2)}s`;
  const who = whoOf(l);
  let what: string;
  if (l.event === "served") {
    what = `served ${ms(l.service_ms ?? 0)}`;
  } else if (l.event === "queued") {
    what = `queued ${ms(l.wait_ms ?? 0)}, served ${ms(l.service_ms ?? 0)}`;
  } else {
    // a failure: what happened, the status the caller saw, and why
    what = [l.event, l.status, l.note].filter((x) => x !== null && x !== "").join(" ");
  }
  // a hop says what kind of hop it was (`proc`, `net`, ...) after the figures
  const kind = l.event === "served" || l.event === "queued" ? (l.note ? ` (${l.note})` : "") : "";
  return `${at} ${l.level.toUpperCase()} r-${l.request} ${l.stream} ${who} ${what}${kind}`;
}

/** `mm:ss.mmm` of virtual time, for the table's time column (tabular figures). */
export function clock(t_ms: number): string {
  const total = Math.max(0, Math.round(t_ms));
  const m = Math.floor(total / 60000);
  const s = Math.floor((total % 60000) / 1000);
  const f = total % 1000;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(f).padStart(3, "0")}`;
}

/** A time in words: a long wait reads in seconds, minutes or hours, not as 247221.6 ms. */
export function dur(n: number | null): string {
  const v = n ?? 0;
  return v >= 1000 ? fmtMs(v) : `${Number(v.toFixed(v < 10 ? 2 : 1))} ms`;
}

/** What happened at the hop, in words: "Waited 210 ms, then served in 98 ms". */
export function eventWords(l: LogLine): string {
  const t = dur;
  // a hop on a wire is a body crossing it: the queue for the wire, then the transfer
  if (l.from) {
    const what = l.note === "link response" ? "Response" : "Request";
    return l.event === "queued"
      ? `${what} waited ${t(l.wait_ms)}, crossed in ${t(l.service_ms)}`
      : `${what} crossed in ${t(l.service_ms)}`;
  }
  if (l.event === "served") return "Served, no wait";
  if (l.event === "queued") return `Waited ${t(l.wait_ms)}, then served in ${t(l.service_ms)}`;
  // "Failed with 503, queue full"; the chaos kind `status` only repeats the status, so it is dropped
  const cause = l.note === "status" || l.note === "" ? null : l.note;
  return `Failed${l.status === null ? "" : ` with ${l.status}`}${cause === null ? "" : `, ${cause}`}`;
}

/** The hop's time in the right-hand column: wait plus service; null for a failure line. */
export function hopMs(l: LogLine): number | null {
  return l.wait_ms === null && l.service_ms === null
    ? null
    : (l.wait_ms ?? 0) + (l.service_ms ?? 0);
}

export interface Waterfall {
  /** Percent of the shared axis, per line, in order. */
  rows: { line: LogLine; label: string; left: number; wait: number; work: number }[];
  /** The one sentence about waiting; null unless some hop has `wait_ms`. */
  finding: string | null;
  /** The failure line, if the request failed (it has no wait split). */
  failure: LogLine | null;
}

/**
 * One request as bars on a shared axis. A hop starts at `t_ms`, waits `wait_ms`, then works
 * `service_ms`. A failure line (R1.88c) has no wait split: it is listed, never drawn as time.
 */
export function waterfall(lines: LogLine[]): Waterfall {
  const hops = lines.filter((l) => l.wait_ms !== null || l.service_ms !== null);
  const failure = lines.find((l) => l.wait_ms === null && l.service_ms === null) ?? null;
  const start = Math.min(...lines.map((l) => l.t_ms));
  const end = Math.max(...lines.map((l) => l.t_ms + (hopMs(l) ?? 0)));
  const span = Math.max(end - start, 0.001);
  const pct = (x: number) => (x / span) * 100;
  const rows = lines.map((line) => ({
    line,
    label: whoOf(line),
    left: pct(line.t_ms - start),
    wait: pct(line.wait_ms ?? 0),
    work: pct(line.service_ms ?? 0),
  }));

  let finding: string | null = null;
  const timed = hops.filter((l) => l.wait_ms !== null);
  if (timed.length > 0) {
    const total = timed.reduce((n, l) => n + (l.wait_ms ?? 0) + (l.service_ms ?? 0), 0);
    const worst = timed.reduce((a, b) => ((b.wait_ms ?? 0) > (a.wait_ms ?? 0) ? b : a));
    const w = worst.wait_ms ?? 0;
    finding =
      total > 0 && w * 2 > total
        ? `Most of it, ${Number(w.toFixed(w < 10 ? 2 : 0))} ms, was spent waiting in line at ${worst.node}`
        : "Little of it was spent waiting in line";
  }
  return { rows, finding, failure };
}

export interface HopType {
  /** The badge's plain-words label. */
  label: string;
  /** `bad` only for a failure; every other hop is a neutral chip. */
  tone: "neutral" | "bad";
}

const KIND: Record<string, string> = {
  proc: "Work",
  net: "Network",
  op: "Data",
  "cache hit": "Cache hit",
  "cache miss": "Cache miss",
  delay: "Delay",
  "link request": "Send",
  "link response": "Reply",
  "node uplink": "Uplink",
  "node downlink": "Downlink",
};
const FAILURE: Record<string, string> = {
  limited: "Rate limited",
  overflowed: "Queue full",
  timeout: "Timed out",
};

/** The type badge of a line: the hop kind the server noted, or on a failure its cause in words. */
export function hopType(l: LogLine): HopType {
  if (l.event !== "served" && l.event !== "queued") {
    return { label: FAILURE[l.event] ?? "Failed", tone: "bad" };
  }
  const note = l.note ?? "";
  // a kind this build does not know is shown as the server wrote it, not guessed at
  const label = KIND[note] ?? (note === "" ? "Work" : note.charAt(0).toUpperCase() + note.slice(1));
  return { label, tone: "neutral" };
}

/** Where a line happened, without the instance: "api", or "gw → api" on a wire. */
export const placeOf = (l: LogLine): string => (l.from ? `${l.from} → ${l.node}` : l.node);

/**
 * How deep each component sits in the call graph: the longest path from a root (a component
 * nothing calls), so gateway 0, balancer 1, api 2, db 3. Cycle-safe: configs are acyclic, but an
 * edge that closes a loop is ignored rather than followed forever.
 */
export function callDepths(
  edges: readonly { from: unknown; to: unknown }[] | null | undefined,
): Map<string, number> {
  const parents = new Map<string, string[]>();
  for (const e of Array.isArray(edges) ? edges : []) {
    if (!e) continue;
    const from = String(e.from);
    const to = String(e.to);
    if (!parents.has(from)) parents.set(from, []);
    parents.set(to, [...(parents.get(to) ?? []), from]);
  }
  const depth = new Map<string, number>();
  const walking = new Set<string>();
  const of = (n: string): number => {
    const known = depth.get(n);
    if (known !== undefined) return known;
    walking.add(n);
    let d = 0;
    for (const p of parents.get(n) ?? []) if (!walking.has(p)) d = Math.max(d, of(p) + 1);
    walking.delete(n);
    depth.set(n, d);
    return d;
  };
  for (const n of parents.keys()) of(n);
  return depth;
}

/** A hop's depth: its component's, or on a wire the caller's plus half a step. */
export function hopDepth(l: LogLine, depths: Map<string, number>): number {
  return l.from ? (depths.get(l.from) ?? 0) + 0.5 : (depths.get(l.node) ?? 0);
}
