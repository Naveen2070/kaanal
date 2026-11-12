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
import { useLogDrawer } from "./logDrawer";
import { PanelBottomOpen } from "lucide-react";

const REFRESH_MS = 2000;
const OPEN_FIRST = 3;

/** `initialEndpoint` preselects the endpoint filter (Admin starts on the real requests). */
export function LiveTrace({
  initialEndpoint = "",
  drawer = false,
}: { initialEndpoint?: string; drawer?: boolean } = {}) {
  const out = useLogDrawer((s) => s.out);
  const setOut = useLogDrawer((s) => s.set);
  const edges = useDraft((s) => s.view?.doc?.edges);
  const nodes = useLive((s) => s.hello?.nodes);
  const streams = useLive((s) => s.hello?.streams);
  const [frozen, setFrozen] = useState(false);
  const [lines, setLines] = useState(() =>
    journeyLines(useLive.getState().ticks, nodes, edges, 40, streams),
  );
  const [toggled, setToggled] = useState<Record<string, boolean>>({});
  useEffect(() => {
    if (frozen) return;
    const take = () => setLines(journeyLines(useLive.getState().ticks, nodes, edges, 40, streams));
    take();
    const t = setInterval(take, REFRESH_MS);
    return () => clearInterval(t);
  }, [frozen, nodes, edges, streams]);
  const depths = useMemo(() => callDepths(edges), [edges]);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [component, setComponent] = useState("");
  const [reqText, setReqText] = useState("");
  const [endpoint, setEndpoint] = useState(initialEndpoint);
  const all = useMemo(() => groupRequests("live", lines), [lines]);
  const names = useMemo(() => [...new Set(lines.map((l) => l.node))].sort(), [lines]);
  // a real mock request (Admin mode) rides in the same sample, named "real"
  const hasReal = initialEndpoint === "real" || lines.some((l) => l.stream === "real");
  const digits = reqText.replace(/\D/g, "");
  const groups = all.filter(
    (g) =>
      (!errorsOnly || g.failure !== null) &&
      (component === "" || g.lines.some((l) => l.node === component)) &&
      (endpoint === "" || g.stream === endpoint) &&
      (digits === "" || String(g.request).includes(digits)),
  );
  const filtered = errorsOnly || component !== "" || endpoint !== "" || digits !== "";
  const finding = sampleFinding(groups);
  // popped out: the list is in the drawer, so the rail says where it went
  if (out && !drawer) {
    return (
      <div className="p-3 text-sm">
        <p className="mb-2 text-muted">The trace is open along the bottom of the window.</p>
        <button
          type="button"
          onClick={() => setOut(false)}
          className="min-h-8 rounded-md border border-control bg-sheet px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          Bring it back here
        </button>
      </div>
    );
  }
  return (
    <div className="p-3 text-sm">
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs font-medium">Live · simulated, newest first</p>
        <div className="flex shrink-0 items-center gap-1">
          {drawer ? null : (
            <button
              type="button"
              title="Open the trace along the bottom of the window, where it has room"
              onClick={() => setOut(true)}
              className="flex items-center gap-1 rounded-md border border-control bg-sheet px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              <PanelBottomOpen size={12} aria-hidden />
              Pop out
            </button>
          )}
          <button
            type="button"
            aria-pressed={frozen}
            onClick={() => setFrozen((f) => !f)}
            className="rounded-md border border-control bg-sheet px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            {frozen ? "Unfreeze" : "Freeze"}
          </button>
        </div>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={errorsOnly}
            onChange={(e) => setErrorsOnly(e.currentTarget.checked)}
          />
          Errors only
        </label>
        <select
          aria-label="Component"
          value={component}
          onChange={(e) => setComponent(e.currentTarget.value)}
          className="h-7 rounded border border-control bg-sheet px-1"
        >
          <option value="">Any component</option>
          {names.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
        {(streams && streams.length > 1) || hasReal ? (
          <select
            aria-label="Endpoint"
            value={endpoint}
            onChange={(e) => setEndpoint(e.currentTarget.value)}
            className="h-7 max-w-40 rounded border border-control bg-sheet px-1"
          >
            <option value="">Any endpoint</option>
            {hasReal ? <option value="real">Real requests (a frontend)</option> : null}
            {(streams ?? []).map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        ) : null}
        <input
          aria-label="Request number"
          placeholder="Request no."
          value={reqText}
          onChange={(e) => setReqText(e.currentTarget.value)}
          className="h-7 w-24 rounded border border-control bg-sheet px-1"
        />
      </div>
      {finding ? <p className="mt-1 text-sm font-medium">{finding}</p> : null}
      <p className="mt-1 mb-2 text-xs text-muted">
        A sample of the last few seconds, not every request. Simulated trace events, not process
        output.
      </p>
      {groups.length > 0 ? <Legend /> : null}
      <div className="@container min-h-40 overflow-hidden rounded-lg border border-border">
        {groups.length === 0 ? (
          <p className="p-3 text-muted">
            {filtered
              ? "No sampled request matches. Clear the filters to see more."
              : "No request finished yet. Raise the offered load or wait."}
          </p>
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
