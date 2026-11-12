// The region menu at the top right (RULES 121): the cloud region of each provider, for the prices
// and for which instance types are offered. One choice for the whole page; the instance picker and
// the cost estimate follow it. A globe button opens a small sheet of three selects.

import { Globe } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { regionOf, useRegion } from "../../data/store/region";

const PROVIDERS: { id: string; label: string }[] = [
  { id: "aws", label: "AWS" },
  { id: "azure", label: "Azure" },
  { id: "gcp", label: "Google Cloud" },
];

export function RegionMenu() {
  const state = useRegion();
  const [open, setOpen] = useState(false);
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const load = state.load;
  useEffect(() => load(), [load]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onDown = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [open]);
  const summary = PROVIDERS.map((p) => `${p.label} ${regionOf(state, p.id) ?? "…"}`).join(", ");
  return (
    <div ref={box} className="relative">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={`${id}-sheet`}
        aria-label={`Cloud region for prices and instance types: ${summary}`}
        title={`Cloud region: ${summary}`}
        onClick={() => setOpen((o) => !o)}
        className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <Globe size={16} aria-hidden />
      </button>
      {open ? (
        <section
          id={`${id}-sheet`}
          aria-label="Cloud region"
          className="absolute top-full right-0 z-30 mt-1 w-64 rounded-xl border border-border bg-sheet p-3 text-sm shadow-float"
        >
          <h2 className="m-0 text-sm font-semibold">Cloud region</h2>
          <p className="m-0 mt-1 mb-2 text-xs text-muted">
            For the cost estimate and for which instance types are offered. A setting of this
            browser, not of the file.
          </p>
          {PROVIDERS.map((p) => (
            <label key={p.id} className="mb-2 flex flex-col gap-0.5 text-xs text-muted last:mb-0">
              {p.label}
              <select
                name={`region-${p.id}`}
                value={regionOf(state, p.id) ?? ""}
                disabled={!state.ready}
                onChange={(e) => state.pick(p.id, e.currentTarget.value)}
                className="h-8 rounded border border-control bg-sheet px-1 text-sm text-text"
              >
                {(state.regions[p.id] ?? []).map((r) => (
                  <option key={r} value={r}>
                    {r}
                  </option>
                ))}
              </select>
            </label>
          ))}
        </section>
      ) : null}
    </div>
  );
}
