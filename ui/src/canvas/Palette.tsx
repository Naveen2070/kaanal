// The node kinds, to drag onto the canvas or (for keyboards and touch) to click. A floating panel
// on the left. Each kind has its own hue on a tinted badge, the same one its card wears. On a wide
// window each kind is its badge and its word; between 900 and 1279 px the panel is a 48px icon rail,
// and the button keeps its name (the kind) for assistive technology and the tooltip.

import { useLayout } from "../shell/layout/useLayout";
import { catColour, NODE_KINDS } from "./nodeKinds";

/** The drag payload's type: what a drop on the canvas reads the kind from. */
export const KIND_MIME = "application/x-kaanal-kind";

export function Palette({ onAdd }: { onAdd: (kind: string) => void }) {
  const rail = useLayout() === "rail";
  return (
    <aside
      data-tour="palette"
      aria-label="Add a node"
      className={`flex shrink-0 flex-col gap-0.5 overflow-y-auto rounded-2xl border border-border bg-sheet p-2 shadow-float ${rail ? "w-12 items-center" : "w-76"}`}
    >
      {rail ? null : (
        <h2 className="px-2 pt-1 pb-1.5 text-xs font-semibold text-muted">Add a node</h2>
      )}
      {NODE_KINDS.map(({ kind, label, icon: Icon, description }) => {
        const hue = catColour(kind);
        return (
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
            className={`group flex min-h-9 cursor-grab items-center gap-2.5 rounded-lg text-left text-sm hover:bg-raised active:cursor-grabbing focus-visible:outline-2 focus-visible:outline-accent ${rail ? "w-9 justify-center" : "w-full px-1.5"}`}
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
      })}
    </aside>
  );
}
