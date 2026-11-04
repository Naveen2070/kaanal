// The loaded lines as requests: one group per request, its hops in time order, with the facts a
// header needs (how long it took, whether it failed, who made it wait) and the one finding for
// the whole sample. Pure, so a page boundary that cuts a request in two is merged by the test, not
// by luck: groups are made from ALL loaded lines every time.

import type { LogLine } from "../../../data/api/types";
import { fmtMs } from "../../../canvas/metrics";
import { dur, hopMs, hopType, placeOf, waterfall, type Waterfall } from "../logFormat";

export interface RequestGroup {
  /** Run, endpoint and request number: the number alone repeats across endpoints. */
  key: string;
  request: number;
  stream: string;
  lines: LogLine[];
  /** The shared axis of this request's bars. */
  axis: Waterfall;
  /** The failure's word ("Queue full"), or null when the request was served. */
  failure: string | null;
  /** Start to end of the request, or null when no hop has a time (only a failure line). */
  total_ms: number | null;
  /** "Waited 9.3 s at api", only when some hop waited. */
  waited: string | null;
}

export function groupRequests(runId: string, lines: readonly LogLine[]): RequestGroup[] {
  const by = new Map<string, LogLine[]>();
  for (const l of lines) {
    const key = `${runId}|${l.stream}|${l.request}`;
    const at = by.get(key);
    if (at) at.push(l);
    else by.set(key, [l]);
  }
  return [...by].map(([key, all]) => {
    // Array.sort is stable: lines at the same instant keep the server's order
    const hops = [...all].sort((a, b) => a.t_ms - b.t_ms);
    const axis = waterfall(hops);
    const failed = hops.find((l) => l.event !== "served" && l.event !== "queued");
    const timed = hops.filter((l) => hopMs(l) !== null);
    const end = Math.max(...timed.map((l) => l.t_ms + (hopMs(l) ?? 0)));
    const worst = hops.reduce<LogLine | null>(
      (a, l) => ((l.wait_ms ?? 0) > (a?.wait_ms ?? 0) ? l : a),
      null,
    );
    const first = hops[0] as LogLine;
    return {
      key,
      request: first.request,
      stream: first.stream,
      lines: hops,
      axis,
      failure: failed ? hopType(failed).label : null,
      total_ms: timed.length === 0 ? null : end - first.t_ms,
      waited:
        worst && (worst.wait_ms ?? 0) > 0
          ? `Waited ${fmtMs(worst.wait_ms ?? 0, 1)} at ${placeOf(worst)}`
          : null,
    };
  });
}

/**
 * The one most useful fact of the loaded sample, from the data and nothing else: where requests
 * failed, else where they waited the longest, else that none waited. Null with nothing to say.
 */
export function sampleFinding(groups: readonly RequestGroup[]): string | null {
  if (groups.length === 0) return null;
  const failed = groups.filter((g) => g.failure !== null);
  if (failed.length > 0) {
    const at = new Map<string, number>();
    for (const g of failed) {
      const l = g.lines.find((x) => x.event !== "served" && x.event !== "queued");
      if (l) at.set(placeOf(l), (at.get(placeOf(l)) ?? 0) + 1);
    }
    const [top = "", n = 0] = [...at].sort((a, b) => b[1] - a[1])[0] ?? [];
    const where = n === failed.length ? `all at ${top}` : `most at ${top}`;
    const noun = failed.length === 1 ? "request" : "requests";
    return `${failed.length.toLocaleString("en-US")} ${noun} failed, ${where}`;
  }
  let worst: LogLine | null = null;
  for (const g of groups)
    for (const l of g.lines) if ((l.wait_ms ?? 0) > (worst?.wait_ms ?? 0)) worst = l;
  if (worst) return `${placeOf(worst)} is where requests waited: ${dur(worst.wait_ms)} at worst`;
  return "No request waited";
}
