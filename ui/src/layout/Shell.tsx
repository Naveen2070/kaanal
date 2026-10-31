// The frame (spec 3): the top bar on the window ground, then the palette, the canvas and the right
// rail as sheets (1px border, 16px radius, no shadow) with an 8px margin and a 12px gap. Under 900 px
// the three give way to the read-only list.

import type { ReactNode } from "react";
import { useLayout } from "./useLayout";

export function Shell({
  top,
  palette,
  canvas,
  rail,
  readonly,
}: {
  top: ReactNode;
  palette: ReactNode;
  canvas: ReactNode;
  rail: ReactNode;
  /** Shown instead of the palette, canvas and rail below 900 px. */
  readonly: ReactNode;
}) {
  const mode = useLayout();
  return (
    <main className="flex h-full flex-col bg-ground text-text">
      {top}
      <div className="flex min-h-0 flex-1 gap-3 px-2 pb-2">
        {mode === "readonly" ? (
          <div className="min-w-0 flex-1 overflow-y-auto rounded-2xl border border-border bg-sheet">
            {readonly}
          </div>
        ) : (
          <>
            {palette}
            <div className="min-w-0 flex-1 overflow-hidden rounded-2xl border border-border bg-sheet">
              {canvas}
            </div>
            {rail}
          </>
        )}
      </div>
    </main>
  );
}
