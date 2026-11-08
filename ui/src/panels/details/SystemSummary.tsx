// The details panel with nothing selected: instead of a blank column, the three components
// nearest to giving out, from the same run figures the cards read (the feed), each a button that
// selects it. With no run, or nothing under load, it says so plainly and keeps the old sentence.

import { useFeed } from "../../canvas/feed";
import { fixFirstLimit } from "../../canvas/insights";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { bandwidthKbOf } from "./edgeFields";
import { closestToBreaking, percent, twoSignificant } from "../../canvas/metrics";
import { useSelected } from "../../data/store/view";
import { HeaderBytes } from "./HeaderBytes";
import { LiveWindow } from "./LiveWindow";
import { TrafficMix } from "./TrafficMix";

export function SystemSummary() {
  const feed = useFeed();
  const run = useRun((s) => s.latest);
  const [, select] = useSelected();
  const rows = closestToBreaking(feed.nodes, 3, feed.links, feed.pipes);
  const loaded = rows.some((r) => r.peak > 0);
  const edges = useDraft((s) => s.view?.doc?.edges);
  const fix = fixFirstLimit(run?.summary ?? undefined, (l) => {
    const e = edges?.find((x) => String(x.from) === l.from && String(x.to) === l.to);
    return e ? bandwidthKbOf(e) : null;
  });
  return (
    <div className="p-3 text-sm">
      <h3 className="m-0 mb-1 text-xs font-medium text-muted">Closest to breaking</h3>
      {rows.length === 0 ? (
        <p className="m-0 text-muted">Run to find out.</p>
      ) : !loaded ? (
        <p className="m-0 text-muted">Nothing is under load yet.</p>
      ) : (
        <ol className="m-0 list-none p-0">
          {rows.map((r) => (
            <li
              key={`${r.wire ? "wire" : "node"} ${r.name}`}
              className="border-b border-rule py-2 last:border-b-0"
            >
              {r.wire ? (
                // a wire is picked on the canvas, so its row states the figure and is not a button
                <div className="flex w-full items-baseline justify-between gap-2">
                  <span className="min-w-0 truncate font-medium">{r.name}</span>
                  <span className="shrink-0 tabular-nums">{percent(r.peak)} full at its peak</span>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => void select(r.name)}
                  className="flex w-full items-baseline justify-between gap-2 rounded-md text-left focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <span className="min-w-0 truncate font-medium">{r.name}</span>
                  <span className="shrink-0 tabular-nums">{percent(r.peak)} busy at its peak</span>
                </button>
              )}
              <p className="m-0 mt-0.5 text-xs text-muted">
                {r.wire ? "Wire · " : ""}
                {r.saturatesAtReqS === null
                  ? "No saturation point"
                  : `Saturates near ${twoSignificant(r.saturatesAtReqS)} req/s`}
              </p>
            </li>
          ))}
        </ol>
      )}
      {fix ? (
        <p className="m-0 mt-2 text-xs">
          <span className="font-medium">To lift the first limit ({fix.name}):</span> {fix.text}
        </p>
      ) : null}
      <p className="m-0 mt-3 text-muted">Select a component to see and change it.</p>
      <TrafficMix />
      <HeaderBytes />
      <LiveWindow />
    </div>
  );
}
