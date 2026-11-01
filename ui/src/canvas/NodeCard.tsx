// A node on the canvas (spec 4): a coloured icon badge (one hue per kind) and the name (a button, so
// a keyboard can select it), a line of config in words, the meter with its sentence, and two
// figures. A card is a step lighter than the panels so it reads as an object on the canvas. Red only
// when the words say saturated. The figures come from the stats feed (feed.ts), never from the run
// store. The settings are not in the card: selecting it opens the dock (ControlDock.tsx), so the
// card never changes size and the edges never move when a component is selected.

import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useEffect } from "react";
import type { NodeDoc } from "../api/types";
import { useFeed } from "./feed";
import { configLine, fmtMs, nodeFigures, stateWords } from "./metrics";
import { Meter } from "./Meter";
import { catColour, kindInfo } from "./nodeKinds";
import { setUpdating, useUpdating } from "./updating";

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
  const node = feed.node(name);
  const f = nodeFigures(node, config);
  const info = kindInfo(kind);
  const Icon = info?.icon;
  const label = info?.label ?? kind;
  const hue = catColour(kind);
  const border = f.saturated ? "border-negative" : selected ? "border-accent" : "border-border";
  // a setting of this node is mid-edit: the meter says "updating" until a run starts or lands
  // (the loop re-runs a result older than the edit, R1.99). Without figures there is nothing to
  // mark, so a draft that never runs cannot leave "updating" stuck on.
  const pending = useUpdating((s) => s.name === name);
  useEffect(() => setUpdating(name, false), [name, feed.dimmed, node]);
  return (
    // a node holds figures, so the card is a group (a fieldset) and only its header is a button
    <fieldset
      aria-label={`${name}, ${label}, ${stateWords(f)}`}
      className={`m-0 w-[236px] min-w-0 overflow-hidden rounded-xl border bg-card p-0 text-text shadow-[0_1px_2px_rgb(0_0_0/0.12)] ${border} ${
        selected ? "outline-2 outline-offset-1 outline-accent" : ""
      }`}
    >
      <div aria-hidden className="h-[3px]" style={{ background: hue }} />
      <div data-part="card" className="relative flex flex-col gap-2 px-3 pt-2.5 pb-3">
        <Handle type="target" position={Position.Left} />
        <button
          type="button"
          className="flex items-center gap-2 rounded-md text-left focus-visible:outline-2 focus-visible:outline-accent"
        >
          {Icon ? (
            <span
              className="flex size-7 shrink-0 items-center justify-center rounded-lg"
              style={{ background: `color-mix(in srgb, ${hue} 16%, transparent)` }}
            >
              <Icon size={16} aria-hidden style={{ color: hue }} />
            </span>
          ) : null}
          <span className="truncate text-sm font-semibold">{name}</span>
        </button>
        <div className="text-xs text-muted">{configLine(label, config)}</div>
        <div className={feed.dimmed ? "opacity-60" : ""}>
          <Meter ratio={f.busy} label={f.sentence} stale={feed.dimmed || (pending && f.hasRun)} />
          <dl className="mt-2 grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-xs">
            <dt className="text-muted">Arriving</dt>
            <dd className="m-0 text-right tabular-nums">{perSecond(f.arrivingPerS)}</dd>
            <dt className="text-muted" title="Time spent working here, not waiting in line">
              Slowest work under
            </dt>
            <dd className="m-0 text-right tabular-nums">
              {f.slowestUnderMs === null ? "–" : fmtMs(f.slowestUnderMs)}
            </dd>
          </dl>
        </div>
        <Handle type="source" position={Position.Right} />
      </div>
    </fieldset>
  );
}
