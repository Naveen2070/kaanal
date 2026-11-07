// The node kinds, to drag onto the canvas or (for keyboards and touch) to click. A floating panel
// on the left. Each kind has its own hue on a tinted badge, the same one its card wears. From 1400 px
// wide each kind is its badge and its word; narrower, the panel is a 56 px icon rail, and each icon
// keeps its name (the kind) for assistive technology and opens a small card on hover or focus with the
// one-line description and a link to the glossary. On the wide panel the foot says how to move
// around, in words that match what the canvas does.

import { StickyNote } from "lucide-react";
import { useId, useState } from "react";
import { useGuide } from "../overlays/guide/store";
import { useCompactPalette } from "../shell/layout/useLayout";
import { addNote } from "./addNote";
import { useDraft } from "../data/store/draft";
import { catColour, offeredKinds } from "./nodeKinds";

/** The drag payload's type: what a drop on the canvas reads the kind from. */
export const KIND_MIME = "application/x-kaanal-kind";

/** The card an icon opens on the rail: the kind's name, what it is, and where to read more. */
function RailCard({
  id,
  label,
  description,
  entry,
  at,
}: {
  id: string;
  label: string;
  description: string;
  /** The glossary entry to open; none for a note. */
  entry?: string;
  /** Where to stand: beside the icon the pointer or focus is on (fixed, so the rail's scroll cannot clip it). */
  at: { top: number; left: number } | null;
}) {
  return (
    <div
      id={id}
      role="tooltip"
      style={at ? { top: at.top, left: at.left } : undefined}
      className="fixed z-30 hidden w-60 -translate-y-1/2 rounded-xl border border-border bg-card p-3 text-left shadow-float group-focus-within:block group-hover:block"
    >
      <b className="block text-[13px]">{label}</b>
      <span className="mt-1 mb-1.5 block text-xs text-muted">{description}</span>
      {entry ? (
        <a
          href="#glossary"
          onClick={(e) => {
            e.preventDefault();
            useGuide.getState().show({ entry });
          }}
          className="text-xs text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent"
        >
          Open in the glossary
        </a>
      ) : null}
    </div>
  );
}

export function Palette({ onAdd }: { onAdd: (kind: string) => void }) {
  const rail = useCompactPalette();
  const kinds = offeredKinds(useDraft((s) => s.view?.kinds));
  const base = useId();
  const [at, setAt] = useState<{ top: number; left: number } | null>(null);
  /** The wrapper of the icon pointed at or focused: its card stands beside it. */
  const stand = (e: { currentTarget: HTMLElement }) => {
    const r = e.currentTarget.getBoundingClientRect();
    setAt({ top: r.top + r.height / 2, left: r.right + 8 });
  };
  return (
    <aside
      data-tour="palette"
      aria-label="Add a node"
      className={`flex shrink-0 flex-col gap-0.5 rounded-2xl border border-border bg-sheet p-2 shadow-float overflow-x-hidden overflow-y-auto ${rail ? "w-14 items-center" : "w-76"}`}
    >
      {rail ? null : (
        <h2 className="px-2 pt-1 pb-1.5 text-xs font-semibold text-muted">Add a node</h2>
      )}
      {kinds.map(({ kind, label, icon: Icon, description }) => {
        const hue = catColour(kind);
        const card = `${base}-${kind}`;
        const button = (
          <button
            type="button"
            draggable
            title={rail ? undefined : description}
            aria-label={rail ? label : undefined}
            aria-describedby={rail ? card : undefined}
            onClick={() => onAdd(kind)}
            onDragStart={(e) => {
              e.dataTransfer.setData(KIND_MIME, kind);
              e.dataTransfer.effectAllowed = "copy";
            }}
            className={`flex min-h-9 cursor-grab items-center gap-2.5 rounded-lg text-left text-sm hover:bg-raised active:cursor-grabbing focus-visible:outline-2 focus-visible:outline-accent ${rail ? "w-10 justify-center" : "w-full px-1.5"}`}
          >
            <span
              className="flex size-7 shrink-0 items-center justify-center rounded-lg"
              style={{ background: `color-mix(in srgb, ${hue} 16%, transparent)` }}
            >
              <Icon size={16} aria-hidden style={{ color: hue }} />
            </span>
            {rail ? null : <span>{label}</span>}
          </button>
        );
        return rail ? (
          <div key={kind} className="group relative" onMouseEnter={stand} onFocus={stand}>
            {button}
            <RailCard id={card} label={label} description={description} entry={kind} at={at} />
          </div>
        ) : (
          <div key={kind}>{button}</div>
        );
      })}
      {rail ? (
        <div className="group relative mt-1" onMouseEnter={stand} onFocus={stand}>
          <button
            type="button"
            aria-label="Add a note"
            aria-describedby={`${base}-note`}
            onClick={() => void addNote()}
            className="flex min-h-9 w-10 items-center justify-center rounded-lg hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-well">
              <StickyNote size={16} aria-hidden className="text-muted" />
            </span>
          </button>
          <RailCard
            id={`${base}-note`}
            label="Add a note"
            description="Pin a caption to the canvas. Notes are kept in this browser, not in the file."
            at={at}
          />
        </div>
      ) : (
        <button
          type="button"
          title="Pin a caption to the canvas. Notes are kept in this browser, not in the file."
          onClick={() => void addNote()}
          className="mt-1 flex min-h-9 w-full items-center gap-2.5 rounded-lg px-1.5 text-left text-sm hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-well">
            <StickyNote size={16} aria-hidden className="text-muted" />
          </span>
          Add a note
        </button>
      )}
      {rail ? null : (
        <p data-testid="canvas-hint" className="m-0 mt-auto px-2 pt-3 pb-1 text-xs text-muted">
          Drag to pan, scroll to zoom. Connect from a card&apos;s right edge. Press ? for shortcuts.
        </p>
      )}
    </aside>
  );
}
