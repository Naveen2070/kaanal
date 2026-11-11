// The live Trace: the journeys the live stream samples (a few a tick, kept ten seconds) turned into
// the same lines the batch Log shows, so the same request blocks draw them. Pure. A journey knows its
// hops and where it was dropped, not the endpoint or the cause, so those say "live" and "dropped here".

import type { Journey, Tick } from "../../data/api/liveTypes";
import type { LogLine } from "../../data/api/types";
import { KIND_LABEL } from "../../data/store/liveMath";

/** Newest first, at most `max` requests. `nodes` is the stream's component order, `edges` the file's. */
export function journeyLines(
  ticks: readonly Tick[],
  nodes: readonly string[] | undefined,
  edges: readonly { from: unknown }[] | undefined,
  max = 40,
  /** The routes of the offered mix, from the stream's `hello`. */
  streams?: readonly string[],
): LogLine[] {
  const out: LogLine[] = [];
  let taken = 0;
  for (let i = ticks.length - 1; i >= 0 && taken < max; i--) {
    const tick = ticks[i] as Tick;
    // a tick's journeys are in finish order, so the later ones are the newer
    for (let j = tick.journeys.length - 1; j >= 0 && taken < max; j--) {
      out.push(...oneJourney(tick, tick.journeys[j] as Journey, j, nodes, edges, streams));
      taken++;
    }
  }
  return out;
}

function oneJourney(
  tick: Tick,
  jr: Journey,
  j: number,
  nodes: readonly string[] | undefined,
  edges: readonly { from: unknown }[] | undefined,
  streams: readonly string[] | undefined,
): LogLine[] {
  // unique per journey: the tick's end time (ms) and its index in the tick
  const request = Math.round(tick.t_ms) * 1000 + j;
  const arrival = tick.t_ms - jr.total_ns / 1e6;
  const stream = jr.real
    ? "real"
    : ((jr.stream !== undefined ? streams?.[jr.stream] : undefined) ?? "live");
  const lines: LogLine[] = [];
  for (const [k, h] of jr.hops.entries()) {
    const node = nodes?.[h.node] ?? `#${h.node}`;
    const wire = h.kind === "net" || h.kind === "link";
    const from = wire && h.edge !== null ? edges?.[h.edge]?.from : undefined;
    const wait = h.wait_ns / 1e6;
    const base = {
      t_ms: arrival + h.start_ns / 1e6,
      request,
      stream,
      node,
      ...(typeof from === "string" ? { from } : {}),
      replica: h.replica,
      ...(h.attempt > 1 ? { attempt: h.attempt } : {}),
    };
    if (jr.dropped_at === k) {
      lines.push({
        ...base,
        level: "error",
        event: "failed",
        wait_ms: null,
        service_ms: null,
        status: null,
        note: jr.cause ? (KIND_LABEL[jr.cause] ?? jr.cause) : "dropped here",
      });
      break;
    }
    lines.push({
      ...base,
      level: "info",
      event: wait > 0 ? "queued" : "served",
      wait_ms: wait,
      service_ms: Math.max(0, (h.end_ns - h.start_ns) / 1e6 - wait),
      status: null,
      note: h.kind,
    });
  }
  return lines;
}
