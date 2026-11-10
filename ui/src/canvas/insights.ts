// Plain-words answers worked out from a finished run, with no server call: why something is red,
// what would fix the first limit, how a run differs from the one before, whether the p99 is within
// a target, a rough plan for a higher rate, and the run as a Markdown report. Pure, so each is
// tested without drawing. Every figure comes from the run summary; nothing is invented.

import type { LinkSummary, NodeSummary, Summary } from "../data/api/types";
import { fmtMs, twoSignificant } from "./metrics";
import { rateText } from "../panels/details/units";

/** A component or wire is planned to run at or under this share of its capacity. */
export const TARGET_BUSY = 0.7;

const nf = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 0 });
const copiesOf = (n: NodeSummary) => Math.max(1, n.replicas.length);

/** How many copies keep `n` at or under `TARGET_BUSY` when its load is `scale` times today's. */
export function copiesNeeded(n: NodeSummary, scale: number): number {
  return Math.max(
    copiesOf(n),
    Math.ceil((copiesOf(n) * (n.offered_load ?? 0) * scale) / TARGET_BUSY),
  );
}

/** Why a saturated component is red, in one or two sentences; null when it is not saturated. */
export function whyNodeRed(n: NodeSummary): string | null {
  if (!n.saturated) return null;
  const cpuBound = n.saturates_by === "cpu" && typeof n.cpu_capacity_per_s === "number";
  const cap = cpuBound ? n.cpu_capacity_per_s : n.capacity_per_s;
  const handles =
    typeof cap === "number" && Number.isFinite(cap) && cap > 0
      ? ` and finishes at most ${nf(cap)}/s`
      : "";
  const waiting = n.max_queue > 0 ? `, so up to ${nf(n.max_queue)} wait in line` : "";
  const away =
    n.limited + n.overflowed > 0 ? `; ${nf(n.limited + n.overflowed)} were turned away` : "";
  const why = cpuBound
    ? " Its CPUs are the limit, not its slots."
    : n.saturates_by === "memory"
      ? " Its memory is the limit: it holds fewer requests at once than its slots allow."
      : "";
  return `${n.name} gets ${twoSignificant(n.visits_per_s)}/s${handles}${waiting}${away}.${why}`;
}

/** The step that would take a saturated component under its target, or null when none is needed. */
export function fixNode(n: NodeSummary): string | null {
  const cpu = n.cpu_load;
  if (n.saturated && typeof cpu === "number" && cpu > 1 && cpu >= (n.offered_load ?? 0)) {
    return `More CPU for ${n.name}: the work asks for ${twoSignificant(cpu)} times the cores it has; about ${twoSignificant(cpu / TARGET_BUSY)} times the cores (or copies) keeps it under ${Math.round(TARGET_BUSY * 100)}% busy.`;
  }
  if (!n.saturated || !((n.offered_load ?? 0) > 0)) return null;
  const now = copiesOf(n);
  const need = copiesNeeded(n, 1);
  return need > now
    ? `Copies of ${n.name}: ${now} now, about ${need} to stay under ${Math.round(TARGET_BUSY * 100)}% busy at this load.`
    : null;
}

/** Why a full wire is red, and why what is behind it looks idle. Null when it is not full. */
export function whyWireRed(l: LinkSummary, bandwidthKb: number | null): string | null {
  if (!l.saturated) return null;
  const asked = Number.isFinite(l.offered_load) ? Math.round(l.offered_load * 100) : null;
  const cap = bandwidthKb !== null ? ` carries ${rateText(bandwidthKb)} but` : "";
  const ask =
    asked !== null ? ` the traffic asks for ${nf(asked)}% of it` : " the traffic asks for more";
  return `The wire ${l.from} → ${l.to}${cap}${ask}, so ${l.to} only sees what the wire lets through and can look idle.`;
}

/** The bandwidth that would keep a full wire at or under the target, in KB/s; null when unknown. */
export function wireNeedKb(l: LinkSummary, bandwidthKb: number | null): number | null {
  if (bandwidthKb === null || !(l.offered_load > 1)) return null;
  return (bandwidthKb * l.offered_load) / TARGET_BUSY;
}

export interface FixLine {
  /** "gateway → shop" for a wire. */
  name: string;
  text: string;
}

/**
 * What to change for the first limit (the lowest saturation point of any component or wire), in a
 * sentence; null when nothing is limiting or the cause has no step we can name.
 */
export function fixFirstLimit(
  s: Summary | undefined,
  bandwidthOf: (l: LinkSummary) => number | null,
): FixLine | null {
  if (!s) return null;
  type Cand = { at: number; name: string; text: string | null };
  const cands: Cand[] = [];
  for (const n of s.nodes) {
    const at = n.saturates_at_req_s;
    if (typeof at === "number" && at > 0) cands.push({ at, name: n.name, text: fixNode(n) });
  }
  for (const l of s.links) {
    const at = l.saturates_at_req_s;
    if (typeof at === "number" && at > 0) {
      const need = wireNeedKb(l, bandwidthOf(l));
      cands.push({
        at,
        name: `${l.from} → ${l.to}`,
        text: need !== null ? `Raise the wire to at least ${rateText(need)} for this load.` : null,
      });
    }
  }
  const first = cands.sort((a, b) => a.at - b.at)[0];
  return first && first.text ? { name: first.name, text: first.text } : null;
}

// ---- compare ----

/** Lines such as "p99 11.7 h → 4.1 s", for the figures that differ between two runs. */
export function compareRuns(prev: Summary, cur: Summary): string[] {
  const out: string[] = [];
  const p = prev.latency.p99_ms;
  const c = cur.latency.p99_ms;
  if (Number.isFinite(p) && Number.isFinite(c) && Math.abs(p - c) > Math.max(0.05 * p, 0.01)) {
    out.push(`p99 ${fmtMs(p, 1)} → ${fmtMs(c, 1)}`);
  }
  const good = (s: Summary) => (s.window_s > 0 ? (s.requests - s.failed) / s.window_s : NaN);
  const gp = good(prev);
  const gc = good(cur);
  if (Number.isFinite(gp) && Number.isFinite(gc) && Math.round(gp) !== Math.round(gc)) {
    out.push(`goodput ${nf(gp)}/s → ${nf(gc)}/s`);
  }
  if (prev.failed !== cur.failed) out.push(`errors ${nf(prev.failed)} → ${nf(cur.failed)}`);
  const first = (s: Summary): string | null => {
    let best: { at: number; name: string } | null = null;
    const see = (at: number | null | undefined, name: string) => {
      if (typeof at === "number" && at > 0 && (best === null || at < best.at)) best = { at, name };
    };
    for (const n of s.nodes) see(n.saturates_at_req_s, n.name);
    for (const l of s.links) see(l.saturates_at_req_s, `${l.from} → ${l.to}`);
    const b = best as { at: number; name: string } | null;
    return b ? `${b.name} at ${twoSignificant(b.at)} req/s` : null;
  };
  const bp = first(prev);
  const bc = first(cur);
  if (bp !== bc) out.push(`breaks first: ${bp ?? "nothing"} → ${bc ?? "nothing"}`);
  return out;
}

// ---- p99 target ----

/** A sentence for the system p99 against a target (both in ms); null without a target. */
export function sloVerdict(p99Ms: number, targetMs: number | null): string | null {
  if (targetMs === null || !(targetMs > 0) || !Number.isFinite(p99Ms)) return null;
  return p99Ms <= targetMs
    ? `p99 ${fmtMs(p99Ms, 1)} is within the ${fmtMs(targetMs, 1)} target (${Math.round((1 - p99Ms / targetMs) * 100)}% to spare).`
    : `p99 ${fmtMs(p99Ms, 1)} is over the ${fmtMs(targetMs, 1)} target by ${fmtMs(p99Ms - targetMs, 1)}.`;
}

// ---- rough plan for a higher rate ----

export interface PlanRow {
  name: string;
  text: string;
}

/**
 * A rough plan for `targetReqS`: today's run scaled in a straight line (a component's load grows
 * with the rate), keeping each component and wire at or under `TARGET_BUSY`. Only what would need
 * more is listed. It does not predict queueing or timeouts; a real run at that rate is the check.
 */
export function planFor(
  s: Summary,
  targetReqS: number,
  bandwidthOf: (l: LinkSummary) => number | null,
): PlanRow[] {
  const rows: PlanRow[] = [];
  for (const n of s.nodes) {
    const at = n.saturates_at_req_s;
    if (typeof at !== "number" || !(at > 0)) continue;
    const now = copiesOf(n);
    const need = Math.max(now, Math.ceil((copiesOf(n) * (targetReqS / at)) / TARGET_BUSY));
    if (need > now) rows.push({ name: n.name, text: `${now} → about ${need} copies` });
  }
  for (const l of s.links) {
    const at = l.saturates_at_req_s;
    const bw = bandwidthOf(l);
    if (typeof at !== "number" || !(at > 0) || bw === null) continue;
    const need = (bw * (targetReqS / at)) / TARGET_BUSY;
    if (need > bw)
      rows.push({
        name: `${l.from} → ${l.to}`,
        text: `${rateText(bw)} → at least ${rateText(need)}`,
      });
  }
  return rows;
}

// ---- report ----

/** The run as Markdown for sharing: what it is, the key figures, what breaks first, warnings. */
export function reportMarkdown(
  runLabel: string,
  s: Summary,
  ranked: { name: string; peak: number; saturatesAtReqS: number | null; wire?: true }[],
): string {
  const good = s.window_s > 0 ? (s.requests - s.failed) / s.window_s : null;
  const lines = [
    `# ${runLabel}`,
    "",
    "_A what-if on a simulated system, not a prediction._",
    "",
    "## Key figures",
    "",
    "| Figure | Value |",
    "| --- | --- |",
    `| p50 | ${fmtMs(s.latency.p50_ms, 1)} |`,
    `| p95 | ${fmtMs(s.latency.p95_ms, 1)} |`,
    `| p99 | ${fmtMs(s.latency.p99_ms, 1)} |`,
    `| Requests | ${nf(s.requests)} |`,
    `| Errors | ${nf(s.failed)} |`,
    ...(good === null ? [] : [`| Goodput | ${nf(good)}/s |`]),
    "",
    "## What breaks first",
    "",
    ...(ranked.length === 0
      ? ["Nothing was under load."]
      : [
          "| Name | Peak | Saturates at |",
          "| --- | --- | --- |",
          ...ranked.map(
            (r) =>
              `| ${r.name}${r.wire ? " (wire)" : ""} | ${Math.round(r.peak * 100)}% | ${
                r.saturatesAtReqS === null
                  ? "no saturation point"
                  : `${twoSignificant(r.saturatesAtReqS)} req/s`
              } |`,
          ),
        ]),
  ];
  if (s.warnings.length > 0) lines.push("", "## Warnings", "", ...s.warnings.map((w) => `- ${w}`));
  const errs = Object.entries(s.errors);
  if (errs.length > 0) {
    lines.push("", "## Errors by kind", "", ...errs.map(([k, v]) => `- ${k}: ${nf(v)}`));
  }
  return lines.join("\n") + "\n";
}
