// The Results tab while Studio is live (step 8L): what breaks first over the window, the timeline
// and the notes. Compare with the last run, plan for a rate, share and the heatmap belong to a
// finished run and stay in Batch.

import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { liveFeed } from "../../canvas/liveFeed";
import { twoSignificant } from "../../canvas/metrics";
import { Section } from "./Field";
import { LiveTimeline } from "./LiveTimeline";

const pct = (n: number) => `${Math.round(n * 100)}%`;

export function LiveResults() {
  const ticks = useLive((s) => s.ticks);
  const hello = useLive((s) => s.hello);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const nodes = liveFeed(ticks, doc, hello?.nodes).nodes;
  // the busiest each component got in the window, not just now
  const peaks = nodes
    .filter((n) => n.utilization !== null)
    .map((n) => ({
      name: n.name,
      peak: Math.max(0, ...n.series.busy),
      queue: Math.max(0, ...n.series.queue),
      saturated: n.saturated,
    }))
    .sort((a, b) => b.peak - a.peak || b.queue - a.queue)
    .slice(0, 6);
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
      <Section title="Over time">
        <LiveTimeline />
      </Section>
    </div>
  );
}
