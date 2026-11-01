// The settings of the selected component (spec 4), floating at the bottom centre of the screen:
// its name and kind, then Time, Slots and Copies side by side. It is not part of the canvas, so it
// keeps one size at every zoom and is never clipped by the canvas edge. Escape closes it.

import { X } from "lucide-react";
import { useDraft } from "../store/draft";
import { useSelected } from "../store/view";
import { ControlBar } from "./ControlBar";
import { catColour, kindInfo } from "./nodeKinds";
import { setUpdating } from "./updating";

export function ControlDock() {
  const [selected, setSelected] = useSelected();
  const config = useDraft((s) => (selected === null ? undefined : s.view?.doc?.nodes?.[selected]));
  if (selected === null || !config) return null;
  const info = kindInfo(config.type);
  const Icon = info?.icon;
  return (
    <section
      aria-label={`Settings for ${selected}`}
      className="pointer-events-auto max-w-full rounded-2xl border border-border bg-sheet px-4 py-3 shadow-float"
    >
      <header className="mb-2 flex items-center gap-2">
        {Icon ? (
          <span
            className="flex size-6 items-center justify-center rounded-md"
            style={{ background: `color-mix(in srgb, ${catColour(config.type)} 16%, transparent)` }}
          >
            <Icon size={14} aria-hidden style={{ color: catColour(config.type) }} />
          </span>
        ) : null}
        <h2 className="m-0 truncate text-sm font-semibold">{selected}</h2>
        <span className="text-xs text-muted">{info?.label ?? config.type}</span>
        <button
          type="button"
          aria-label="Close settings"
          onClick={() => void setSelected(null)}
          className="ml-auto flex size-6 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <X size={14} aria-hidden />
        </button>
      </header>
      <ControlBar
        dock
        name={selected}
        config={config}
        onPending={(on) => setUpdating(selected, on)}
        onClose={() => void setSelected(null)}
      />
    </section>
  );
}
