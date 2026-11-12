// The Results tab while Studio is live (step 8L): what breaks first over the window, the timeline
// and the notes. Compare with the last run, plan for a rate, share and the heatmap belong to a
// finished run and stay in Batch.

import { useEffect, useState } from "react";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { endpointFigures, errorKinds, LIVE_BUCKETS } from "../../data/store/liveMath";
import { measureDraft } from "../../data/store/measure";
import { useRun } from "../../data/store/run";
import { useSelected } from "../../data/store/view";
import { BusyChart } from "./BusyChart";
import { Heatmap } from "./Heatmap";
import { Results } from "./Results.lazy";
import { liveFeed } from "../../canvas/liveFeed";
import { breaksFirst, fmtMs, twoSignificant } from "../../canvas/metrics";
import { Section } from "./Field";
import { LiveTimeline } from "./LiveTimeline";

const pct = (n: number) => `${Math.round(n * 100)}%`;

export function LiveResults() {
  const ticks = useLive((s) => s.ticks);
  const hello = useLive((s) => s.hello);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const nodes = liveFeed(ticks, doc, hello?.nodes).nodes;
  const [selected] = useSelected();
  const kinds = errorKinds(ticks);
  const run = useRun((x) => x.latest);
  const status = useRun((x) => x.status);
  const [problem, setProblem] = useState<string | null>(null);
  const measuring = status === "running";
  // the rate on offer now: the measured run is made at it, so it compares with the live figures
  const offered = ticks.at(-1)?.status.rate ?? hello?.rate ?? 0;
  const measure = async () => setProblem(await measureDraft(offered > 0 ? offered : undefined));
  // optionally measure again a few seconds after the draft changes, so "measured" follows the edits
  const [auto, setAuto] = useState(false);
  const rev = useDraft((s) => s.view?.rev ?? -1);
  useEffect(() => {
    if (!auto) return;
    const t = setTimeout(() => void measureDraft(offered > 0 ? offered : undefined), 3000);
    return () => clearTimeout(t);
    // the offered rate is read when the timer fires; only a new draft revision restarts it
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auto, rev]);
  const measuredLimit = breaksFirst(run?.summary ?? undefined);
  const routes = hello?.streams ?? [];
  const routeFigures = routes.map((name, i) => ({ name, ...endpointFigures(ticks, i) }));
  const order = hello?.nodes ?? Object.keys(doc?.nodes ?? {});
  // the busiest each component got in the window, not just now
  const peaks = nodes
    .filter((n) => n.utilization !== null)
    .map((n) => ({
      name: n.name,
      peak: Math.max(0, ...n.series.busy),
      queue: Math.max(0, ...n.series.queue),
      saturated: n.saturated,
      limit: n.saturates_at_req_s,
    }))
    .sort((a, b) => b.peak - a.peak || b.queue - a.queue)
    .slice(0, 6);
  // the component picked on the canvas, else the one that peaked highest
  const picked =
    nodes.find((n) => n.name === selected) ?? nodes.find((n) => n.name === peaks[0]?.name);
  return (
    <div className="p-3 text-sm">
      <p className="mb-2 text-xs text-muted">
        The live run: what the system is doing, not a finished measurement. Run once in Batch for a
        repeatable one.
      </p>
      {hello?.note ? <p className="mb-2 text-xs text-warn">{hello.note}</p> : null}
      <Section title="What breaks first">
        {peaks.length === 0 ? (
          <p className="text-xs text-muted">No component has a figure yet.</p>
        ) : (
          <ol className="m-0 list-none p-0 text-xs">
            {peaks.map((r, i) => (
              <li key={r.name} className="flex h-11 flex-col justify-center">
                <div className="flex items-baseline gap-2">
                  <span className="min-w-0 flex-1 truncate font-medium">{r.name}</span>
                  <span className="tabular-nums text-muted">peak {pct(r.peak)}</span>
                </div>
                <div className="tabular-nums text-[11px] text-muted">
                  {r.queue > 0 ? `up to ${twoSignificant(r.queue)} waiting` : "nothing waiting"}
                  {r.saturated ? " · saturated now" : ""}
                  {r.limit !== null
                    ? ` · limit about ${twoSignificant(r.limit)} req/s (estimate)`
                    : ""}
                </div>
                <div aria-hidden className="h-1 rounded-full bg-well">
                  <div
                    className={`h-1 rounded-full ${i === 0 && r.peak >= 0.7 ? "bg-bad-fill" : "bg-text"}`}
                    style={{ width: `${Math.min(r.peak, 1) * 100}%` }}
                  />
                </div>
              </li>
            ))}
          </ol>
        )}
      </Section>
      {picked ? (
        <Section title={`Busy over time: ${picked.name}`}>
          <BusyChart
            name={picked.name}
            busy={picked.series.busy}
            windowS={picked.series.window_s}
          />
        </Section>
      ) : null}
      {picked && picked.series.latency.some((w) => w.some((c) => c > 0)) ? (
        <Section title={`Latency over the last minute: ${picked.name}`} help="latency-map">
          <Heatmap
            name={picked.name}
            series={{ window_s: 1, latency: picked.series.latency }}
            buckets={LIVE_BUCKETS}
          />
        </Section>
      ) : null}
      {picked && picked.replicas.length > 1 ? (
        <Section title={`Instances of ${picked.name}`}>
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left">
                <th>Instance</th>
                <th>Visits/s</th>
                <th>Running</th>
                <th>Waiting</th>
              </tr>
            </thead>
            <tbody>
              {picked.replicas.map((r) => {
                const load =
                  ticks.at(-1)?.nodes[order.indexOf(picked.name)]?.copy_load?.[r.replica];
                return (
                  <tr key={r.replica}>
                    <td>{r.replica}</td>
                    <td className="tabular-nums">{twoSignificant(r.visits_per_s)}</td>
                    <td className="tabular-nums">{load ? load[0] : "–"}</td>
                    <td className="tabular-nums">{load ? load[1] : "–"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <p className="mt-1 text-[11px] text-muted">
            Running and waiting are the slots of each copy at this instant.
          </p>
        </Section>
      ) : null}
      {routeFigures.length > 1 ? (
        <Section title="By route (last 10 seconds)">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left">
                <th>Route</th>
                <th>Req/s</th>
                <th>Failed/s</th>
                <th>p50</th>
                <th>p99</th>
              </tr>
            </thead>
            <tbody>
              {routeFigures.map((r) => (
                <tr key={r.name}>
                  <td className="max-w-32 truncate" title={r.name}>
                    {r.name}
                  </td>
                  <td className="tabular-nums">{twoSignificant(r.reqPerS)}</td>
                  <td className="tabular-nums">{twoSignificant(r.failedPerS)}</td>
                  <td className="tabular-nums">{r.p50Ms === null ? "–" : fmtMs(r.p50Ms)}</td>
                  <td className="tabular-nums">{r.p99Ms === null ? "–" : fmtMs(r.p99Ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
      ) : null}
      <Section title="Errors by kind">
        {kinds.length === 0 ? (
          <p className="text-xs text-muted">Nothing failed in this window.</p>
        ) : (
          <ul className="m-0 list-none p-0 text-xs">
            {kinds.map(([kind, n]) => (
              <li key={kind} className="flex justify-between py-0.5">
                <span>{kind}</span>
                <span className="font-mono tabular-nums">{n.toLocaleString("en-US")}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="Over time">
        <LiveTimeline />
      </Section>
      <Section title="Measure">
        <p className="mb-2 text-xs text-muted">
          One finished, repeatable run of the draft{" "}
          {offered > 0
            ? `at the rate on offer now (${twoSignificant(offered)} / s)`
            : "at the rate in its traffic profile"}
          : the verdict, heatmap, compare, plan and share, as in Batch. It does not change the live
          run.
        </p>
        {measuredLimit ? (
          <p className="mb-2 text-xs">
            Measured limit: <strong>{measuredLimit.node}</strong> at about{" "}
            {twoSignificant(measuredLimit.atReqS)} / s (from the run below, so it follows the
            settings it was run with).
          </p>
        ) : null}
        <button
          type="button"
          disabled={measuring || doc?.nodes === undefined}
          onClick={() => void measure()}
          className="h-8 rounded-lg border border-control bg-sheet px-3 text-xs font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
        >
          {measuring ? "Measuring..." : run ? "Measure again" : "Measure the draft"}
        </button>
        <label className="mt-2 flex items-center gap-1.5 text-xs text-muted">
          <input
            type="checkbox"
            checked={auto}
            onChange={(e) => setAuto(e.currentTarget.checked)}
          />
          Measure again 3 seconds after each change, while this tab is open
        </label>
        {problem ? <p className="mt-2 text-xs text-warn">{problem}</p> : null}
      </Section>
      {run ? (
        <div className="-mx-3 mt-2 border-t border-border">
          <LazyBoundary fallback={null}>
            <Results />
          </LazyBoundary>
        </div>
      ) : null}
    </div>
  );
}
