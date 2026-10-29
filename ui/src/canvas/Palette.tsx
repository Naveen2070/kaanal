// The node kinds, to drag onto the canvas or (for keyboards and touch) to click.

import { NODE_KINDS } from "./nodeKinds";

/** The drag payload's type: what a drop on the canvas reads the kind from. */
export const KIND_MIME = "application/x-kaanal-kind";

export function Palette({ onAdd }: { onAdd: (kind: string) => void }) {
  return (
    <aside
      data-tour="palette"
      aria-label="Add a node"
      className="flex w-48 flex-col gap-2 border-r p-2"
    >
      <h2 className="text-xs font-semibold uppercase text-neutral-500">Add a node</h2>
      {NODE_KINDS.map(({ kind, label, icon: Icon, accent, description }) => (
        <button
          key={kind}
          type="button"
          draggable
          title={description}
          onClick={() => onAdd(kind)}
          onDragStart={(e) => {
            e.dataTransfer.setData(KIND_MIME, kind);
            e.dataTransfer.effectAllowed = "copy";
          }}
          className={`flex cursor-grab items-center gap-2 rounded-md border-2 px-2 py-1 text-left text-sm ${accent}`}
        >
          <Icon size={16} aria-hidden />
          <span className="text-neutral-900 dark:text-neutral-100">{label}</span>
        </button>
      ))}
    </aside>
  );
}
