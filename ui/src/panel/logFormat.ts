// One line of the run log as text, like the spec's example:
//   t=12.31s WARN r-4812 POST /orders api#2 queued 210ms, served 98ms
// The stamp is VIRTUAL time inside the simulation, and the lines are simulated trace events, not
// process output (the tab says so). `#2` after a node name is display only.

import type { LogLine } from "../api/types";

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
