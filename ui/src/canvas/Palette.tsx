// The node kinds, to drag onto the canvas or (for keyboards and touch) to click. On a wide window
// each kind is an icon and its word; between 900 and 1279 px the panel is a 48px icon rail, and the
// button keeps its name (the kind) for assistive technology and the tooltip.

import { useLayout } from "../layout/useLayout";
import { NODE_KINDS } from "./nodeKinds";

/** The drag payload's type: what a drop on the canvas reads the kind from. */
export const KIND_MIME = "application/x-kaanal-kind";

export function Palette({ onAdd }: { onAdd: (kind: string) => void }) {
  const rail = useLayout() === "rail";
  return (
    <aside
      data-tour="palette"
      aria-label="Add a node"
      className={`flex shrink-0 flex-col gap-1 overflow-y-auto rounded-2xl border border-border bg-sheet p-2 ${rail ? "w-12 items-center" : "w-[232px]"}`}
    >
      {rail ? null : <h2 className="px-2 py-1 text-xs font-semibold text-muted">Add a node</h2>}
      {NODE_KINDS.map(({ kind, label, icon: Icon, description }) => (
        <button
          key={kind}
          type="button"
          draggable
          title={rail ? `${label}: ${description}` : description}
          aria-label={rail ? label : undefined}
          onClick={() => onAdd(kind)}
          onDragStart={(e) => {
            e.dataTransfer.setData(KIND_MIME, kind);
            e.dataTransfer.effectAllowed = "copy";
          }}
          className={`flex min-h-8 cursor-grab items-center gap-2 rounded-lg text-left text-sm hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent ${rail ? "w-8 justify-center" : "w-full px-2"}`}
        >
          <Icon size={16} aria-hidden className="shrink-0 text-muted" />
          {rail ? null : <span>{label}</span>}
        </button>
      ))}
    </aside>
  );
}
