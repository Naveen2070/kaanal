// What a card shows, worked out from one node's run figures and its config. Pure, so the numbers
// are tested without drawing anything. Honest metrics (spec section 6): the busy bar is measured
// utilization, the peak is the highest of the ~60 windows, the sparkline is queue depth (exact),
// and headroom is capacity, not a prediction.

import type { LinkSummary, NodeDoc, NodeSummary, PipeSummary, Summary } from "../data/api/types";
import { durationMs, hasProc, slotsOf, worksOutTo } from "../panels/details/fields";

export interface Figures {
  /** Measured utilization 0..1; null without a run or without slots. */
  busy: number | null;
  /** The highest windowed busy fraction; null without a busy series. */
  peak: number | null;
  /** Longest queue per window. */
  queue: number[];
  /** The longest queue seen in the run. */
  waiting: number | null;
  /** "saturates near 120 req/s"; null without a capacity. */
  headroomText: string | null;
  saturated: boolean;
  /** Copies of the node (1 when the config says nothing). */
  replicas: number;
  /** True once a run has given any figure for this node. */
  hasRun: boolean;
  /** The longest queue of the run (a peak, never a live count); 0 without a run. */
  peakWaiting: number;
  /** Requests per second reaching the node; null without a run. */
  arrivingPerS: number | null;
  /** The node's p99 hop work time, exact over the run: "slowest work under N ms". */
  slowestUnderMs: number | null;
  /** Requests turned away here (rate limit or full queue) in the whole run. */
  turnedAway: number;
  /** Turned away per second, scaled from the arrival rate; null when nothing got in to scale by. */
  turnedAwayPerS: number | null;
  /** One line for the card, the details panel and anything else that says it in words. */
  sentence: string;
}

/** Two significant digits: 122.4 gives 120, 1530 gives 1500, 7.46 gives 7.5. */
export function twoSignificant(n: number): string {
  return Number(n.toPrecision(2)).toLocaleString("en-US");
}

/** Milliseconds for a person: "32 ms" up to a second, then "15.3 s", "4.2 min" and "15 h". */
export function fmtMs(ms: number, digits = 0): string {
  const one = (n: number, unit: string) =>
    `${n.toLocaleString("en-US", { maximumFractionDigits: 1 })} ${unit}`;
  if (ms >= 3_600_000) return one(ms / 3_600_000, "h");
  if (ms >= 60_000) return one(ms / 60_000, "min");
  if (ms >= 1000) return one(ms / 1000, "s");
  return `${ms.toLocaleString("en-US", { maximumFractionDigits: digits })} ms`;
}

export function replicasOf(config: NodeDoc | undefined): number {
  const r = config?.replicas;
  return typeof r === "number" && Number.isInteger(r) && r >= 1 ? r : 1;
}

export function nodeFigures(node: NodeSummary | undefined, config: NodeDoc | undefined): Figures {
  const replicas = replicasOf(config);
  if (!node) {
    return {
      busy: null,
      peak: null,
      queue: [],
      waiting: null,
      headroomText: null,
      saturated: false,
      replicas,
      hasRun: false,
      peakWaiting: 0,
      arrivingPerS: null,
      slowestUnderMs: null,
      turnedAway: 0,
      turnedAwayPerS: null,
      sentence: "No run yet",
    };
  }
  const busy = node.series.busy;
  const req = node.saturates_at_req_s;
  const peakWaiting = finiteOrNull(node.max_queue) ?? 0;
  const arrivingPerS = finiteOrNull(node.visits_per_s);
  const turnedAway = node.limited + node.overflowed;
  const visits = node.replicas.reduce((n, r) => n + r.visits, 0);
  return {
    busy: node.utilization,
    peak: busy.length > 0 ? Math.max(...busy) : null,
    queue: node.series.queue,
    waiting: node.max_queue,
    headroomText: req !== null && req > 0 ? `saturates near ${twoSignificant(req)} req/s` : null,
    saturated: node.saturated,
    replicas,
    hasRun: true,
    peakWaiting,
    arrivingPerS,
    slowestUnderMs: finiteOrNull(node.p99_ms),
    turnedAway,
    turnedAwayPerS:
      turnedAway > 0 && visits > 0 && arrivingPerS !== null
        ? (arrivingPerS * turnedAway) / visits
        : null,
    sentence: sentenceOf(node, peakWaiting, arrivingPerS),
  };
}

const nf = (n: number, digits = 0) => n.toLocaleString("en-US", { maximumFractionDigits: digits });

/**
 * "Can finish up to 1,280/s · 32 at once ÷ 25 ms" (with copies: "3 × 32 at once"). Local arithmetic
 * on the config, so the control bar shows it live while a slider moves; null without a time.
 */
export function capacityLine(
  slots: number | null,
  ms: number | null,
  copies: number,
): string | null {
  if (ms === null) return null;
  if (slots === null) return "No slot limit · never saturates";
  const at = `${copies > 1 ? `${nf(copies)} × ` : ""}${nf(slots)} at once ÷ ${nf(ms, 3)} ms`;
  if (ms <= 0) return `No limit · ${at}`;
  const perS = worksOutTo(slots, ms, copies).capacityPerS;
  return `Can finish up to ${perS < 10 ? twoSignificant(perS) : nf(perS)}/s · ${at}`;
}

const finiteOrNull = (x: number | null | undefined): number | null =>
  typeof x === "number" && Number.isFinite(x) ? x : null;

/** "78% busy · up to 12 waiting". The words carry the state; colour only repeats it. */
function sentenceOf(node: NodeSummary, peakWaiting: number, arriving: number | null): string {
  if (node.utilization === null) return "No slot limit · never saturates";
  const waiting =
    peakWaiting > 0 ? `up to ${peakWaiting.toLocaleString("en-US")} waiting` : "nothing waiting";
  const turning = node.limited + node.overflowed > 0;
  // turned away while the slots are not all taken: the queue is what is full, so say that
  if (turning && node.utilization < 0.9) {
    return `${percent(node.utilization)} busy · turning requests away`;
  }
  if (node.saturated) return `Saturated · ${waiting}${turning ? " · turning requests away" : ""}`;
  if (arriving === 0 && node.utilization === 0) return "Idle";
  return `${percent(node.utilization)} busy · ${waiting}`;
}

export interface BreaksFirst {
  /** The component's name, or for a wire "from → to". */
  node: string;
  atReqS: number;
  saturatedNow: boolean;
  /** Set when the limit is a wire's bandwidth, not a component's slots. */
  wire?: true;
}

/**
 * What gives out first as load grows: the lowest saturation point of any component or any wire
 * with a bandwidth (null without one). A wire counts because a thin link starves the components
 * behind it long before their own slots fill.
 */
export function breaksFirst(summary: Summary | undefined): BreaksFirst | null {
  let best: BreaksFirst | null = null;
  const consider = (candidate: BreaksFirst) => {
    if (best === null || candidate.atReqS < best.atReqS) best = candidate;
  };
  const usable = (at: number | null | undefined): at is number =>
    typeof at === "number" && Number.isFinite(at) && at > 0;
  for (const n of summary?.nodes ?? []) {
    const at = n.saturates_at_req_s;
    if (usable(at)) consider({ node: n.name, atReqS: at, saturatedNow: n.saturated });
  }
  for (const l of summary?.links ?? []) {
    const at = l.saturates_at_req_s;
    if (usable(at)) {
      consider({ node: `${l.from} → ${l.to}`, atReqS: at, saturatedNow: l.saturated, wire: true });
    }
  }
  for (const p of summary?.pipes ?? []) {
    const at = p.saturates_at_req_s;
    if (usable(at)) consider({ node: pipeName(p), atReqS: at, saturatedNow: p.saturated });
  }
  return best;
}

/** "app send pipe": a node's network card, one direction, named for a ranking. */
export const pipeName = (p: PipeSummary): string => `${p.node} ${p.direction} pipe`;

/** A pipe's peak: demand over capacity for the run, at most 100% (as a wire's). */
const pipePeak = (p: PipeSummary): number => Math.min(1, Math.max(p.offered_load, p.utilization));

/** A whole percent, or an en dash when there is nothing to show. */
export function percent(x: number | null): string {
  return x === null ? "–" : `${Math.round(x * 100)}%`;
}

/** "Service · 1 copy × 32 slots · 25 ms": what the node is set to, in words. A database or cache has no Time. */
export function configLine(label: string, config: NodeDoc | undefined): string {
  const copies = replicasOf(config);
  const slots = config ? slotsOf(config).value : null;
  const parts = [
    label,
    `${copies} ${copies === 1 ? "copy" : "copies"}${
      slots === null
        ? " · no slot limit"
        : ` × ${slots.toLocaleString("en-US")} ${slots === 1 ? "slot" : "slots"}`
    }`,
  ];
  if (config && hasProc(String(config.type)) && "proc" in config) {
    const ms = durationMs(config.proc);
    parts.push(ms === null ? "varies" : `${Number(ms.toFixed(3))} ms`);
  }
  if (config?.type === "storage") {
    const fb = config.first_byte;
    const median = typeof fb === "object" && fb !== null ? durationMs((fb as NodeDoc).p50) : null;
    const ms = durationMs(fb) ?? median;
    parts.push(
      ms === null
        ? "first byte varies"
        : `first byte ${median !== null ? "p50 " : ""}${Number(ms.toFixed(3))} ms`,
    );
  }
  return parts.join(" · ");
}

/** The state in words for a screen reader: "78 percent busy", "saturated", "idle", "no run yet". */
export function stateWords(f: Figures): string {
  if (!f.hasRun) return "no run yet";
  if (f.busy === null) return "no slot limit";
  if (f.saturated) return "saturated";
  if (f.arrivingPerS === 0 && f.busy === 0) return "idle";
  return `${Math.round(f.busy * 100)} percent busy`;
}

export interface Ranked {
  name: string;
  peak: number;
  saturatesAtReqS: number | null;
  /** A wire (named "from → to"), not a component. */
  wire?: true;
  /** The CPUs, not the slots, set the saturation point (step 8). */
  cpuBound?: true;
}

/**
 * The Results tab's "What breaks first": components and wires with a bandwidth, by their peak
 * utilisation, highest first. Ties keep the server's bottleneck order, components before wires. A
 * component with no slot limit has no peak and is left out.
 */
export function rankedByPeak(summary: Summary): Ranked[] {
  const order = (i: number) => {
    const at = summary.bottlenecks.indexOf(i);
    return at === -1 ? Number.MAX_SAFE_INTEGER : at;
  };
  const nodes = summary.nodes.map((n, i) => {
    const busy = n.series.busy.filter(Number.isFinite);
    const slotPeak = busy.length > 0 ? Math.max(...busy) : n.utilization;
    // a component with cores but no slot limit still has a peak: its CPU load, at most 100%
    const cpu = finiteOrNull(n.cpu_load ?? null);
    const cpuPeak = cpu === null ? null : Math.min(1, cpu);
    const peak =
      slotPeak !== null && cpuPeak !== null ? Math.max(slotPeak, cpuPeak) : (slotPeak ?? cpuPeak);
    return {
      i,
      name: n.name,
      peak,
      saturatesAtReqS: finiteOrNull(n.saturates_at_req_s),
      wire: false,
      cpuBound: n.saturates_by === "cpu",
    };
  });
  const wires = summary.links.map((l, k) => {
    // demand over capacity for the whole run, at most 100%: a wire's own windows cannot be trusted
    // when it is full (the records leave out the requests it spends the run serving)
    return {
      i: summary.nodes.length + k,
      name: `${l.from} → ${l.to}`,
      peak: Math.min(1, Math.max(l.offered_load, l.utilization)),
      saturatesAtReqS: finiteOrNull(l.saturates_at_req_s),
      wire: true,
      cpuBound: false,
    };
  });
  const pipes = (summary.pipes ?? []).map((p, k) => ({
    i: summary.nodes.length + summary.links.length + k,
    name: pipeName(p),
    peak: pipePeak(p),
    saturatesAtReqS: finiteOrNull(p.saturates_at_req_s),
    wire: false,
    cpuBound: false,
  }));
  return [...nodes, ...wires, ...pipes]
    .filter((r): r is typeof r & { peak: number } => r.peak !== null)
    .sort((a, b) => b.peak - a.peak || order(a.i) - order(b.i) || a.i - b.i)
    .map(({ name, peak, saturatesAtReqS, wire, cpuBound }) => ({
      name,
      peak,
      saturatesAtReqS,
      ...(wire && { wire: true as const }),
      ...(cpuBound && { cpuBound: true as const }),
    }));
}

/** A rate in three significant figures, so "229,327" reads "229,000": more digits than that are noise. */
const rate = (n: number) => (n < 10 ? twoSignificant(n) : nf(Number(n.toPrecision(3))));

/**
 * The details panel's sentence (spec 5.1), in plain words:
 * "Handles up to 1,280 requests a second. It gets 1,000, so it is 78% busy and up to 12 wait."
 */
export function plainSentence(node: NodeSummary | undefined): string {
  if (!node) return "No run yet.";
  if (node.utilization === null) return "It has no slot limit, so it never saturates.";
  const gets = finiteOrNull(node.visits_per_s);
  const cap = finiteOrNull(node.capacity_per_s);
  const waiting = finiteOrNull(node.max_queue) ?? 0;
  const state = node.saturated ? "saturated" : `${percent(node.utilization)} busy`;
  const tail = waiting > 0 ? ` and up to ${nf(waiting)} wait` : "";
  const handles = cap !== null && cap > 0 ? `Handles up to ${rate(cap)} requests a second. ` : "";
  const getsText = gets !== null ? `It gets ${rate(gets)}, so it is ` : "It is ";
  return `${handles}${getsText}${state}${tail}.`;
}

/**
 * The components nearest to giving out, for the empty details panel: by their peak busy share over
 * the run, highest first, at most `n`. A component with no slot limit has no peak and is left out;
 * ties keep the run's order.
 */
export function closestToBreaking(
  nodes: NodeSummary[],
  n = 3,
  links: LinkSummary[] = [],
  pipes: PipeSummary[] = [],
): { name: string; peak: number; saturatesAtReqS: number | null; wire?: LinkSummary }[] {
  const comps = nodes.map((node, i) => {
    const busy = node.series.busy.filter(Number.isFinite);
    const peak = busy.length > 0 ? Math.max(...busy) : node.utilization;
    return {
      i,
      name: node.name,
      peak,
      saturatesAtReqS: finiteOrNull(node.saturates_at_req_s),
      wire: undefined as LinkSummary | undefined,
    };
  });
  // a wire's peak is demand over capacity for the run, at most 100% (as in rankedByPeak)
  const wires = links.map((l, k) => ({
    i: nodes.length + k,
    name: `${l.from} → ${l.to}`,
    peak: Math.min(1, Math.max(l.offered_load, l.utilization)) as number | null,
    saturatesAtReqS: finiteOrNull(l.saturates_at_req_s),
    wire: l as LinkSummary | undefined,
  }));
  const cards = pipes.map((p, k) => ({
    i: nodes.length + links.length + k,
    name: pipeName(p),
    peak: pipePeak(p) as number | null,
    saturatesAtReqS: finiteOrNull(p.saturates_at_req_s),
    wire: undefined as LinkSummary | undefined,
  }));
  return [...comps, ...wires, ...cards]
    .filter((r): r is typeof r & { peak: number } => r.peak !== null)
    .sort((a, b) => b.peak - a.peak || a.i - b.i)
    .slice(0, n)
    .map(({ name, peak, saturatesAtReqS, wire }) => ({
      name,
      peak,
      saturatesAtReqS,
      ...(wire && { wire }),
    }));
}
