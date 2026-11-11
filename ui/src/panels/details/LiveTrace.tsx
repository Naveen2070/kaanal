// The Trace tab while Studio is live: a sample of the requests the running system finished in the last
// seconds, as the same request blocks the batch Log draws. It refreshes every couple of seconds, not
// with every tick, so a block is not gone while it is being read; Freeze holds the list still.

import { useEffect, useMemo, useState } from "react";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { callDepths } from "./logFormat";
import { ColumnHeads, Legend } from "./log/Legend";
import { groupRequests, sampleFinding } from "./log/groups";
import { RequestBlock } from "./log/RequestBlock";
import { journeyLines } from "./journeyLines";

const REFRESH_MS = 2000;
const OPEN_FIRST = 3;

export function LiveTrace() {
  const edges = useDraft((s) => s.view?.doc?.edges);
  const nodes = useLive((s) => s.hello?.nodes);
  const [frozen, setFrozen] = useState(false);
  const [lines, setLines] = useState(() => journeyLines(useLive.getState().ticks, nodes, edges));
  const [toggled, setToggled] = useState<Record<string, boolean>>({});
  useEffect(() => {
    if (frozen) return;
    const take = () => setLines(journeyLines(useLive.getState().ticks, nodes, edges));
    take();
    const t = setInterval(take, REFRESH_MS);
    return () => clearInterval(t);
  }, [frozen, nodes, edges]);
  const depths = useMemo(() => callDepths(edges), [edges]);
  const groups = useMemo(() => groupRequests("live", lines), [lines]);
  const finding = sampleFinding(groups);
  return (
    <div className="p-3 text-sm">
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs font-medium">Live · simulated, newest first</p>
        <button
          type="button"
          aria-pressed={frozen}
          onClick={() => setFrozen((f) => !f)}
          className="rounded-md border border-control bg-sheet px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          {frozen ? "Unfreeze" : "Freeze"}
        </button>
      </div>
      {finding ? <p className="mt-1 text-sm font-medium">{finding}</p> : null}
      <p className="mt-1 mb-2 text-xs text-muted">
        A sample of the last few seconds, not every request. Simulated trace events, not process
        output.
      </p>
      {groups.length > 0 ? <Legend /> : null}
      <div className="@container min-h-40 overflow-hidden rounded-lg border border-border">
        {groups.length === 0 ? (
          <p className="p-3 text-muted">No request finished yet. Raise the offered load or wait.</p>
        ) : (
          <>
            <ColumnHeads />
            <ol className="m-0 list-none p-0">
              {groups.map((g, i) => (
                <RequestBlock
                  key={g.key}
                  group={g}
                  depths={depths}
                  open={toggled[g.key] ?? i < OPEN_FIRST}
                  onToggle={() =>
                    setToggled((t) => ({ ...t, [g.key]: !(t[g.key] ?? i < OPEN_FIRST) }))
                  }
                />
              ))}
            </ol>
          </>
        )}
      </div>
    </div>
  );
}
