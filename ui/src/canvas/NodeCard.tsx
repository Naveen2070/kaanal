// A node on the canvas (spec 4): a grey kind icon and the name (a button, so a keyboard can select
// it), a line of config in words, the meter with its sentence, and two figures. Red only when the
// words say saturated. The figures come from the stats feed (feed.ts), never from the run store.

import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { NodeDoc } from "../api/types";
import { useFeed } from "./feed";
import { configLine, nodeFigures, stateWords } from "./metrics";
import { Meter } from "./Meter";
import { kindInfo } from "./nodeKinds";
import { QuickControls } from "./QuickControls";

export interface CardData extends Record<string, unknown> {
  /** The node's real name (the React Flow id is its encoded form). */
  name: string;
  kind: string;
  /** The node as the draft writes it. */
  config: NodeDoc;
}

/** 1,000/s; one decimal under 10/s; an en dash without a figure. */
function perSecond(n: number | null): string {
  if (n === null) return "–";
  return `${n.toLocaleString("en-US", { maximumFractionDigits: n < 10 ? 1 : 0 })}/s`;
}

export function NodeCard({ data, selected }: NodeProps) {
  const { name, kind, config } = data as CardData;
  const feed = useFeed();
  const f = nodeFigures(feed.node(name), config);
  const info = kindInfo(kind);
  const Icon = info?.icon;
  const label = info?.label ?? kind;
  const border = f.saturated ? "border-negative" : selected ? "border-accent" : "border-border";
  return (
    // a node holds fields, so the card is a group (a fieldset) and only its header is a button
    <fieldset
      aria-label={`${name}, ${label}, ${stateWords(f)}`}
      className={`m-0 w-[236px] min-w-0 rounded-xl border bg-sheet p-0 text-text ${border} ${
        selected ? "outline-2 outline-accent" : ""
      }`}
    >
      <Handle type="target" position={Position.Left} />
      <div className="flex flex-col gap-2 px-3 pt-3 pb-3">
        <button
          type="button"
          className="flex items-center gap-2 rounded-md text-left focus-visible:outline-2 focus-visible:outline-accent"
        >
          {Icon ? <Icon size={16} aria-hidden className="shrink-0 text-muted" /> : null}
          <span className="truncate text-sm font-semibold">{name}</span>
        </button>
        <div className="text-xs text-muted">{configLine(label, config)}</div>
        <div className={feed.dimmed ? "opacity-60" : ""}>
          <Meter ratio={f.busy} label={f.sentence} stale={feed.dimmed} />
          <dl className="mt-2 grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-xs">
            <dt className="text-muted">Arriving</dt>
            <dd className="m-0 text-right tabular-nums">{perSecond(f.arrivingPerS)}</dd>
            <dt className="text-muted">Slowest requests under</dt>
            <dd className="m-0 text-right tabular-nums">
              {f.slowestUnderMs === null ? "–" : `${f.slowestUnderMs.toLocaleString("en-US")} ms`}
            </dd>
          </dl>
        </div>
        <QuickControls name={name} config={config} />
      </div>
      <Handle type="source" position={Position.Right} />
    </fieldset>
  );
}
