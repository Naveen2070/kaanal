// A node on the canvas (spec 4): a grey kind icon and the name (a button, so a keyboard can select
// it), a line of config in words, the meter with its sentence, and two figures. Red only when the
// words say saturated. The figures come from the stats feed (feed.ts), never from the run store.
// The control bar opens under the card on selection or keyboard focus within it, never on hover
// (a card that grows under the pointer makes edges jump while panning, WCAG 1.4.13). The handles
// sit in the card part, so opening the bar does not move the edges.

import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useRef, useState } from "react";
import type { NodeDoc } from "../api/types";
import { ControlBar } from "./ControlBar";
import { useFeed } from "./feed";
import { configLine, nodeFigures, stateWords } from "./metrics";
import { Meter } from "./Meter";
import { kindInfo } from "./nodeKinds";

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
  const header = useRef<HTMLButtonElement>(null);
  const [focused, setFocused] = useState(false);
  /** Escape closed the bar; it stays closed until the node is selected again or focus leaves. */
  const [dismissed, setDismissed] = useState(false);
  /** A slider is moving or its edit is on the way: the meter says "updating" until the run starts. */
  const [pending, setPending] = useState(false);
  const [wasSelected, setWasSelected] = useState(selected);
  if (wasSelected !== selected) {
    setWasSelected(selected);
    if (selected) setDismissed(false);
  }
  // the feed's own marker takes over when the run state next changes: a run starting for this
  // edit, or one landing (the loop re-runs a result older than the edit, R1.99). Without figures
  // there is nothing to mark, so a draft that never runs cannot leave "updating" stuck on.
  const node = feed.node(name);
  const [seen, setSeen] = useState({ dimmed: feed.dimmed, node });
  if (seen.dimmed !== feed.dimmed || seen.node !== node) {
    setSeen({ dimmed: feed.dimmed, node });
    setPending(false);
  }
  const open = (selected || focused) && !dismissed;
  return (
    // a node holds fields, so the card is a group (a fieldset) and only its header is a button
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- focus within, from its controls
    <fieldset
      aria-label={`${name}, ${label}, ${stateWords(f)}`}
      className={`m-0 w-[236px] min-w-0 rounded-xl border bg-sheet p-0 text-text ${border} ${
        selected ? "outline-2 outline-accent" : ""
      }`}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
        setFocused(false);
        setDismissed(false);
      }}
    >
      <div data-part="card" className="relative flex flex-col gap-2 px-3 pt-3 pb-3">
        <Handle type="target" position={Position.Left} />
        <button
          ref={header}
          type="button"
          onClick={() => setDismissed(false)}
          className="flex items-center gap-2 rounded-md text-left focus-visible:outline-2 focus-visible:outline-accent"
        >
          {Icon ? <Icon size={16} aria-hidden className="shrink-0 text-muted" /> : null}
          <span className="truncate text-sm font-semibold">{name}</span>
        </button>
        <div className="text-xs text-muted">{configLine(label, config)}</div>
        <div className={feed.dimmed ? "opacity-60" : ""}>
          <Meter ratio={f.busy} label={f.sentence} stale={feed.dimmed || (pending && f.hasRun)} />
          <dl className="mt-2 grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-xs">
            <dt className="text-muted">Arriving</dt>
            <dd className="m-0 text-right tabular-nums">{perSecond(f.arrivingPerS)}</dd>
            <dt className="text-muted">Slowest requests under</dt>
            <dd className="m-0 text-right tabular-nums">
              {f.slowestUnderMs === null ? "–" : `${f.slowestUnderMs.toLocaleString("en-US")} ms`}
            </dd>
          </dl>
        </div>
        <Handle type="source" position={Position.Right} />
      </div>
      {open ? (
        <ControlBar
          name={name}
          config={config}
          onPending={setPending}
          onClose={() => {
            header.current?.focus();
            setDismissed(true);
          }}
        />
      ) : null}
    </fieldset>
  );
}
