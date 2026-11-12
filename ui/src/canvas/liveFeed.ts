// The live stats feed (step 8L): the cards read the same `StatsFeed` they read for a finished run
// (canvas/feed.ts, the "4b seam"), built here from the last seconds of live ticks. Per component:
// what arrived, what was turned away, how full its slots or cores are right now, its queue, and a
// sparkline of the last minute. No wire has a bandwidth figure in the live feed (those need a
// batch run), so the cards' wire halos follow the nodes only.

import type { Doc, NodeDoc, NodeSummary, Series } from "../data/api/types";
import type { Tick } from "../data/api/liveTypes";
import { slotsOf } from "../panels/details/fields";
import { EMPTY_FEED, type StatsFeed } from "./feed";
import { quantileMs, nodeHeat, nodeHist } from "../data/store/liveMath";
import { replicasOf } from "./metrics";

/** Ticks the per-second figures are read over (5 seconds at four a second). */
const FIGURE_TICKS = 20;
/** Ticks in a card's sparkline (one minute). */
const SERIES_TICKS = 240;
const TICK_S = 0.25;

const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

/** How full a node is at one tick: its slots in use, else its cores in use (at most 1); null with neither. */
function fullness(
  t: Tick | undefined,
  i: number,
  slots: number | null,
  copies: number,
): number | null {
  const n = t?.nodes[i];
  if (!n) return null;
  if (slots !== null && slots > 0) return Math.min(1, n.busy_slots / (slots * copies));
  return n.cpu_load === null ? null : Math.min(1, n.cpu_load);
}

function nodeSummary(name: string, i: number, cfg: NodeDoc, ticks: readonly Tick[]): NodeSummary {
  const slots = slotsOf(cfg).value;
  const copies = replicasOf(cfg);
  const w = ticks.slice(-FIGURE_TICKS);
  const secs = Math.max(w.length * TICK_S, TICK_S);
  const last = ticks.at(-1);
  const at = (t: Tick) => t.nodes[i];
  const arrived = sum(w.map((t) => at(t)?.arrived ?? 0));
  const drop = (
    k: "overflow" | "rate_limit" | "deadline" | "breaker" | "oom" | "chaos" | "pinned",
  ) => sum(w.map((t) => at(t)?.dropped[k] ?? 0));
  const dropped =
    drop("overflow") + drop("rate_limit") + drop("deadline") + drop("breaker") + drop("oom");
  const util = fullness(last, i, slots, copies);
  // The utilization law (Denning & Buzen, 1978): U = X x S, so a node that is U busy at a system
  // rate X is busy 1 at X / U, if its work per request stays as it is. Read over the last minute
  // (the instant a tick samples is noisy at light load) and only from 1% busy up; an estimate that
  // holds while the mix and the settings stay as they are.
  const minute = ticks.slice(-SERIES_TICKS);
  const meanFull =
    minute.reduce((a, t) => a + (fullness(t, i, slots, copies) ?? 0), 0) /
    Math.max(minute.length, 1);
  const sysPerS = sum(minute.map((t) => t.requests)) / Math.max(minute.length * TICK_S, TICK_S);
  const saturatesAt = meanFull >= 0.01 && sysPerS > 0 ? sysPerS / meanFull : null;
  const queue = last?.nodes[i]?.queue ?? 0;
  const series: Series = {
    window_s: TICK_S,
    busy: ticks.slice(-SERIES_TICKS).map((t) => fullness(t, i, slots, copies) ?? 0),
    queue: ticks.slice(-SERIES_TICKS).map((t) => at(t)?.queue ?? 0),
    // per-node work times: a minute of them as windows (the heatmap), none with an older server
    latency: nodeHeat(ticks.slice(-SERIES_TICKS), i),
  };
  const hist = nodeHist(w, i);
  // each copy's own hops over the window; padded to the copies the node has, so a lane that has
  // not run yet still shows as idle
  const perCopy = w.reduce<number[]>((acc, t) => {
    (t.nodes[i]?.replicas ?? []).forEach((n, r) => (acc[r] = (acc[r] ?? 0) + n));
    return acc;
  }, []);
  const replicas =
    perCopy.length === 0
      ? []
      : Array.from({ length: Math.max(copies, perCopy.length) }, (_, r) => ({
          replica: r,
          visits: perCopy[r] ?? 0,
          visits_per_s: (perCopy[r] ?? 0) / secs,
          mean_wait_ms: 0,
          offered_load: null,
          utilization: null,
        }));
  return {
    name,
    replicas,
    visits_per_s: arrived / secs,
    expected_visits_per_s: arrived / secs,
    mean_wait_ms: 0,
    offered_load: util,
    utilization: util,
    saturated: dropped > 0 || queue > 0 || (util !== null && util >= 0.95),
    max_queue: Math.max(0, ...w.map((t) => at(t)?.queue ?? 0)),
    capacity_per_s: null,
    saturates_at_req_s: saturatesAt,
    headroom: util === null ? null : Math.max(0, 1 - util),
    p50_ms: quantileMs(hist, 0.5),
    p99_ms: quantileMs(hist, 0.99),
    series,
    // failures that are not capacity (a node taken down, a pinned error): the card says so
    failed_per_s: (drop("chaos") + drop("pinned")) / secs,
    limited: drop("rate_limit"),
    overflowed: drop("overflow"),
    timed_out: drop("deadline"),
    short_circuited: drop("breaker"),
    oom_kills: drop("oom"),
    cpu_load: last?.nodes[i]?.cpu_load ?? null,
    cpu_utilization: last?.nodes[i]?.cpu_utilization ?? null,
  };
}

const NO_DROPS = {
  overflow: 0,
  rate_limit: 0,
  deadline: 0,
  breaker: 0,
  oom: 0,
  chaos: 0,
  pinned: 0,
};

/**
 * The ticks as if only the real (frontend) requests had been counted: arrivals, drops and totals
 * come from `tick.real`; the queue, the slots in use and the CPU load are the engine's snapshot and
 * stay, whichever requests made them. Only the newest ticks, which is all a card reads.
 */
function realOnly(ticks: readonly Tick[]): Tick[] {
  return ticks.slice(-SERIES_TICKS).map((t) => ({
    ...t,
    requests: t.real?.requests ?? 0,
    failed: t.real?.failed ?? 0,
    nodes: t.nodes.map((n, i) => ({
      ...n,
      arrived: t.real?.nodes[i]?.arrived ?? 0,
      dropped: t.real?.nodes[i]?.dropped ?? NO_DROPS,
    })),
    wires: t.wires.map((_, i) => t.real?.wires[i] ?? { carried: 0 }),
    hist: t.real?.hist ?? [],
  }));
}

/** The feed of the last seconds of the live run; the empty feed before the first tick. */
export function liveFeed(
  all: readonly Tick[],
  doc: Doc | null,
  /** The components in the order of the ticks' `nodes` (from the stream's `hello`). */
  order?: readonly string[],
  /** Which requests the figures count (Admin mode's "Real only"); all by default. */
  src: "all" | "real" = "all",
): StatsFeed {
  const ticks = src === "real" ? realOnly(all) : all;
  if (ticks.length === 0 || !doc?.nodes) return EMPTY_FEED;
  const docNames = Object.keys(doc.nodes);
  // a tick's nodes are in file order but the draft's JSON is alphabetical: map by name, not by position
  const names = order && order.length > 0 ? [...order] : docNames;
  const nodes = names
    .filter((name) => doc.nodes?.[name] !== undefined)
    .map((name, _k) =>
      nodeSummary(
        name,
        names.indexOf(name),
        (doc.nodes as Record<string, NodeDoc>)[name] as NodeDoc,
        ticks,
      ),
    );
  return {
    node: (name) => nodes.find((n) => n.name === name),
    links: [],
    pipes: [],
    nodes,
    dimmed: false,
    peakVisits: Math.max(0, ...nodes.map((n) => n.visits_per_s).filter(Number.isFinite)),
  };
}
