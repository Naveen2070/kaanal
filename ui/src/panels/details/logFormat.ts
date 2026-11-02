// One line of the run log as text, like the spec's example:
//   t=12.31s WARN r-4812 POST /orders api#2 queued 210ms, served 98ms
// The stamp is VIRTUAL time inside the simulation, and the lines are simulated trace events, not
// process output (the tab says so). `#2` after a node name is display only.

import type { LogLine } from "../../data/api/types";

function ms(n: number): string {
  return `${Number(n.toFixed(n < 10 ? 2 : 1))}ms`;
}

export function formatLine(l: LogLine): string {
  const at = `t=${(l.t_ms / 1000).toFixed(2)}s`;
  const who = l.replica === null ? l.node : `${l.node}#${l.replica}`;
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

/** What happened at the hop, in words: "Waited 210 ms, then served in 98 ms". */
export function eventWords(l: LogLine): string {
  const t = (n: number | null) => `${Number((n ?? 0).toFixed((n ?? 0) < 10 ? 2 : 1))} ms`;
  if (l.event === "served") return "Served, no wait";
  if (l.event === "queued") return `Waited ${t(l.wait_ms)}, then served in ${t(l.service_ms)}`;
  return ["Failed", l.status, l.note].filter((x) => x !== null && x !== "").join(" ");
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
    label: line.replica === null ? line.node : `${line.node}#${line.replica}`,
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
