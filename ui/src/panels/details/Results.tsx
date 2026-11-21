// The Results tab (spec 7.2): the report label and its warnings, what breaks first (components and
// wires), a heatmap for one component or one wire (latency, or time on the wire), per-instance and
// per-link figures, errors by kind.

import { useId, useState } from "react";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { p99WarningFor } from "../../canvas/p99Warning";
import { strictnessNote } from "../../canvas/strictness";
import { useChaosHints } from "../../data/store/chaosHints";
import { useSelected } from "../../data/store/view";
import { BusyChart } from "./BusyChart";
import { Heatmap } from "./Heatmap";
import { BreakerTimeline, RetryFunnel } from "./ResilienceCharts";
import { Section } from "./Field";
import { fmtMs, rankedByPeak, twoSignificant } from "../../canvas/metrics";
import { ResultsSkeleton, RANK_ROW } from "./Skeletons";
import { CompareLine, PlanSection, ReportButtons, SloSection, Verdict } from "./ResultsExtras";
import { Group } from "./groups/Group";
import { RESULTS_GROUP } from "./groups/ids";
import { useSlo } from "./slo";
import { SweepGroup } from "../sweep/SweepGroup";

/** Starts the value of a wire in the picker, so it cannot be a component's name. */
const WIRE = String.fromCharCode(31);

const pct = (x: number | null) => (x === null ? "–" : `${Math.round(x * 100)}%`);

export function Results() {
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  // a pick holds only for the selection it was made under: selecting on the canvas moves the heatmap
  // a key is a component's name, or a wire's place in the run's list behind a control character
  const [picked, setPicked] = useState<{ key: string; under: string | null } | null>(null);
  const dimmed = useRun((s) => s.dimmed);
  const status = useRun((s) => s.status);
  const pickId = useId();
  const doc = useDraft((st) => st.view?.doc);
  const hints = useChaosHints();
  const s = run?.summary;
  const target = useSlo((x) => x.targetMs);

  // the sweep does not need a finished run: it shows (and keeps a finished sweep) on every path
  if (!s) {
    return (
      <>
        {status === "running" ? (
          <ResultsSkeleton />
        ) : (
          <p className="p-3 text-sm text-muted">
            No finished run yet. The results appear after the first run.
          </p>
        )}
        <div className="px-3 pb-3 text-sm">
          <SweepGroup />
        </div>
      </>
    );
  }

  const slow = s.requests > 0 ? p99WarningFor(s.latency.p99_ms, doc) : null;
  const strict = strictnessNote(doc);

  // the heatmap follows the selected component, else the first bottleneck, else the first node
  const ranked = rankedByPeak(s);
  const names = s.nodes.map((n) => n.name);
  const fallback = s.bottlenecks[0] !== undefined ? s.nodes[s.bottlenecks[0]]?.name : names[0];
  const wireKeys = s.links.map((_, k) => `${WIRE}${k}`);
  const keys = [...names, ...wireKeys];
  const pick = picked && picked.under === selected ? picked.key : null;
  const chosenKey =
    [pick, selected, fallback].find(
      (k): k is string => typeof k === "string" && keys.includes(k),
    ) ?? keys[0];
  const link = chosenKey?.startsWith(WIRE) ? s.links[Number(chosenKey.slice(1))] : undefined;
  const node = link ? undefined : s.nodes.find((n) => n.name === chosenKey);

  return (
    <div className="p-3 text-sm">
      {/* always present: a number from this tool is a relative what-if, never a prediction */}
      {/* only a blocked try "could not finish": a run in flight, or one waiting out a busy slot
          (a sweep holds it), is dimmed without the warning */}
      {dimmed && status === "blocked" ? (
        <p className="mb-2 text-xs text-warn">
          Out of date: the newest run could not finish, so these are the last figures that did.
        </p>
      ) : null}
      <p className="mb-2 text-xs font-medium">
        {run ? `Run ${run.id.replace(/^s/, "")} · ` : ""}A what-if, not a prediction
      </p>
      <Verdict s={s} />
      <CompareLine runId={run?.id} s={s} />
      {slow !== null ? <p className="mb-2 text-xs text-warn">{slow}</p> : null}
      {hints.map((h) => (
        <p key={h.node} className="mb-2 text-xs text-muted">
          {h.text}
        </p>
      ))}
      {strict !== null ? <p className="mb-2 text-xs text-muted">{strict}</p> : null}
      <Section title="What breaks first" help="saturation">
        {ranked.length === 0 ? (
          <p className="text-xs text-muted">No traffic reached any component or wire.</p>
        ) : (
          <ol className="text-xs">
            {ranked.map((r, i) => (
              <li key={r.name} className={`flex flex-col justify-center ${RANK_ROW}`}>
                <div className="flex items-baseline gap-2">
                  <span
                    title={r.wire ? `${r.name} (a wire)` : r.name}
                    className="min-w-0 flex-1 truncate font-medium"
                  >
                    {r.name}
                  </span>
                  <span className="tabular-nums text-muted">peak {pct(r.peak)}</span>
                </div>
                {/* two measures stack in one row: the name over its figure, the limit under it */}
                <div className="tabular-nums text-[11px] text-muted">
                  {r.wire ? "Wire · " : ""}
                  {r.cpuBound ? "CPU bound · " : ""}
                  {r.saturatesAtReqS !== null
                    ? `saturates at ${twoSignificant(r.saturatesAtReqS)} req/s`
                    : "no saturation point"}
                </div>
                <div aria-hidden className="h-1 rounded-full bg-well">
                  <div
                    className={`h-1 rounded-full ${i === 0 ? "bg-bad-fill" : "bg-text"}`}
                    style={{ width: `${Math.min(r.peak, 1) * 100}%` }}
                  />
                </div>
              </li>
            ))}
          </ol>
        )}
      </Section>

      <Section title="Latency over the run" help="latency-map">
        {node || link ? (
          <>
            <label htmlFor={pickId} className="mb-1 block text-xs">
              {s.links.length > 0 ? "Component or wire" : "Component"}{" "}
              <select
                name="pick"
                id={pickId}
                value={chosenKey}
                onChange={(e) => setPicked({ key: e.currentTarget.value, under: selected })}
                className="max-w-full rounded border border-control bg-sheet text-text px-1"
              >
                <optgroup label="Components">
                  {names.map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </optgroup>
                {s.links.length > 0 ? (
                  <optgroup label="Wires">
                    {s.links.map((l, k) => (
                      <option key={wireKeys[k]} value={wireKeys[k]}>
                        {l.from} → {l.to}
                      </option>
                    ))}
                  </optgroup>
                ) : null}
              </select>
            </label>
            {link ? (
              <Heatmap
                wire
                name={`${link.from} → ${link.to}`}
                series={link.series}
                buckets={s.latency.buckets}
              />
            ) : node ? (
              <>
                <Heatmap name={node.name} series={node.series} buckets={s.latency.buckets} />
                <BusyChart
                  name={node.name}
                  busy={node.series.busy}
                  windowS={node.series.window_s}
                />
              </>
            ) : null}
          </>
        ) : (
          <p className="text-xs text-muted">This run has no components.</p>
        )}
      </Section>

      {node && node.replicas.length > 1 ? (
        <Section title={`Instances of ${node.name}`}>
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left">
                <th>Instance</th>
                <th>Visits/s</th>
                <th>Busy</th>
                <th>Wait</th>
              </tr>
            </thead>
            <tbody>
              {node.replicas.map((r) => (
                <tr key={r.replica}>
                  <td>{r.replica}</td>
                  <td>{twoSignificant(r.visits_per_s)}</td>
                  <td>{pct(r.utilization)}</td>
                  <td>{twoSignificant(r.mean_wait_ms)} ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
      ) : null}

      {s.links.length > 0 ? (
        <Section title="Links">
          <ul className="space-y-1 text-xs">
            {s.links.map((l) => (
              <li key={`${l.from}>${l.to}`}>
                {l.from} to {l.to}: {pct(l.utilization)} busy, {fmtMs(l.mean_wait_ms)} wait,{" "}
                {l.saturated ? "saturated" : "ok"}
                {l.saturates_at_req_s !== null
                  ? `, fills at ${twoSignificant(l.saturates_at_req_s)} req/s`
                  : ""}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      <RetryFunnel retries={s.retries ?? []} />
      <BreakerTimeline breakers={s.breakers ?? []} windowS={s.window_s} />

      <Section title="Errors by kind" help="errors-kind">
        {Object.keys(s.errors).length === 0 ? (
          <p className="text-xs tabular-nums">0</p>
        ) : (
          <ul className="text-xs">
            {Object.entries(s.errors).map(([kind, n]) => (
              <li key={kind}>
                {kind}: {n}
              </li>
            ))}
          </ul>
        )}
      </Section>

      <SweepGroup />

      <Group
        id={RESULTS_GROUP.goals}
        title="Goals"
        summary={target === null ? "no p99 target" : `p99 target ${fmtMs(target)}`}
      >
        <SloSection s={s} />
        <PlanSection s={s} />
      </Group>

      <Group id={RESULTS_GROUP.share} title="Share this run">
        <ReportButtons runId={run?.id} s={s} />
      </Group>

      {s.warnings.length > 0 ? (
        <Group
          id={RESULTS_GROUP.notes}
          title="Engine notes"
          summary={`${s.warnings.length} ${s.warnings.length === 1 ? "note" : "notes"}`}
        >
          <ul className="mb-2 space-y-1 rounded-lg border border-warn p-2 text-xs text-warn">
            {s.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </Group>
      ) : null}
    </div>
  );
}
