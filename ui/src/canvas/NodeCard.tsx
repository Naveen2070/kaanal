// A node on the canvas: icon and name, a busy bar with a peak tick, the queue sparkline, the
// waiting count, the headroom line, and the quick controls. Red when saturated, and it says so in
// words. The figures come from the stats feed (feed.ts), never from the run store.

import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { NodeDoc } from "../api/types";
import { useFeed } from "./feed";
import { nodeFigures, percent } from "./metrics";
import { kindInfo } from "./nodeKinds";
import { QuickControls } from "./QuickControls";
import { Sparkline } from "./Sparkline";

export interface CardData extends Record<string, unknown> {
  /** The node's real name (the React Flow id is its encoded form). */
  name: string;
  kind: string;
  /** The node as the draft writes it. */
  config: NodeDoc;
}

export function NodeCard({ data, selected }: NodeProps) {
  const { name, kind, config } = data as CardData;
  const feed = useFeed();
  const f = nodeFigures(feed.node(name), config);
  const info = kindInfo(kind);
  const Icon = info?.icon;
  const tone = f.saturated ? "border-red-600 bg-red-50 text-red-700" : (info?.accent ?? "");
  return (
    <div
      className={`min-w-48 rounded-lg border-2 bg-white px-3 py-2 shadow-sm dark:bg-neutral-900 ${tone} ${
        selected ? "ring-2 ring-blue-500" : ""
      } ${feed.dimmed ? "opacity-60" : ""}`}
    >
      <Handle type="target" position={Position.Left} />
      <div className="flex items-center gap-2">
        {Icon ? <Icon size={16} aria-hidden /> : null}
        <span className="truncate font-medium text-neutral-900 dark:text-neutral-100">{name}</span>
        {f.replicas > 1 ? <span className="text-xs text-neutral-500">x {f.replicas}</span> : null}
      </div>
      <div className="text-xs text-neutral-500">{info?.label ?? kind}</div>

      <div className="mt-1 flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300">
        {/* the bar is for the eye; the meter is what a screen reader gets */}
        {f.busy === null ? (
          <span className="sr-only">Busy: no run yet</span>
        ) : (
          <meter
            className="sr-only"
            aria-label="Busy"
            min={0}
            max={100}
            value={Math.round(f.busy * 100)}
          >
            {percent(f.busy)}
          </meter>
        )}
        <div aria-hidden className="relative h-2 flex-1 rounded bg-neutral-200">
          {f.busy !== null ? (
            <div
              className={`h-full rounded ${f.saturated ? "bg-red-600" : "bg-emerald-500"}`}
              style={{ width: `${Math.min(100, f.busy * 100)}%` }}
            />
          ) : null}
          {f.peak !== null ? (
            <div
              className="absolute top-[-2px] h-3 w-0.5 bg-neutral-800"
              style={{ left: `${Math.min(100, f.peak * 100)}%` }}
              title={`peak ${percent(f.peak)}`}
            />
          ) : null}
        </div>
        <span className="w-9 text-right tabular-nums">{percent(f.busy)}</span>
      </div>

      <div className="flex items-center justify-between text-xs text-neutral-700 dark:text-neutral-300">
        <Sparkline values={f.queue} />
        <span title="longest queue">waiting {f.waiting ?? "–"}</span>
      </div>
      <div className="text-xs text-neutral-500">{f.headroomText ?? "–"}</div>
      {f.saturated ? (
        <div className="text-xs font-semibold">
          Saturated: the queue grows or work is turned away
        </div>
      ) : null}

      <QuickControls name={name} config={config} />
      <Handle type="source" position={Position.Right} />
    </div>
  );
}
