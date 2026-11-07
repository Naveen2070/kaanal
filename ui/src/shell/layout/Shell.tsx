// The frame (spec 3). The canvas is the page: it fills the window, grid lines and all, and every
// other part floats over it as a sheet: the title card at the top left, the key figures at the top centre, the theme
// and help at the top right, the palette down the left, the details rail down the right, and in the
// middle the notices at the top, the dock and the charts at the bottom, with the zoom pill beside
// the rail. The empty middle lets pointer events through to the canvas. Under 768 px there is no
// canvas to edit: the top bar and a read-only list.

import type { CSSProperties, ReactNode } from "react";
import { useCompactPalette, useLayout, useWide } from "./useLayout";

export function Shell({
  top,
  topLeft,
  topRight,
  palette,
  canvas,
  rail,
  dock,
  bottom,
  zoom,
  toasts,
  readonly,
}: {
  /** The key figures: a sheet at the top, centred between the two cards beside it. */
  top: ReactNode;
  /** Title, Run, Apply and Save: a small stacked card at the top left. */
  topLeft: ReactNode;
  /** Theme and help: a small sheet at the top right. */
  topRight?: ReactNode;
  palette: ReactNode;
  canvas: ReactNode;
  rail: ReactNode;
  /** The selected component's settings, floating at the bottom centre. */
  dock?: ReactNode;
  /** The charts panel, below the dock. */
  bottom?: ReactNode;
  /** Zoom and full screen, on the bottom row beside the rail. */
  zoom?: ReactNode;
  /** The notices, at the top of the free space. */
  toasts?: ReactNode;
  /** Shown instead of the palette, canvas and rail below 768 px. */
  readonly: ReactNode;
}) {
  const mode = useLayout();
  const compactPalette = useCompactPalette();
  const editing = mode !== "readonly";
  // on a wide screen the rail stands in its own column from the top, under the theme box, so the
  // room beside the title card is not left empty; narrower, the figures card needs that width
  const wide = useWide();
  const column = editing && wide;
  // under 1280 px the key figures get a row of their own below the title card and the icons
  const stacked = editing && !column && mode !== "full";
  return (
    <main
      className="relative h-full overflow-hidden bg-ground text-text"
      style={{ "--palette-w": compactPalette ? "56px" : "304px" } as CSSProperties}
    >
      {/* the canvas comes first in the page, so a keyboard gets a way past its wires and cards */}
      {editing ? (
        <a
          href="#studio-top"
          onClick={(e) => {
            e.preventDefault();
            document.getElementById("studio-top")?.focus();
          }}
          className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-lg focus:bg-primary focus:px-3 focus:py-2 focus:text-sm focus:text-on-primary"
        >
          Skip the canvas
        </a>
      ) : null}
      {editing ? <div className="absolute inset-0">{canvas}</div> : null}
      <div
        className={`relative z-10 flex h-full gap-3 p-3 ${
          editing ? "pointer-events-none" : "flex-col overflow-y-auto"
        }`}
      >
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <header
            id="studio-top"
            tabIndex={-1}
            className={`shrink-0 outline-none ${
              !editing
                ? "flex flex-col gap-2"
                : column
                  ? "grid grid-cols-[auto_minmax(0,1fr)] items-start gap-3"
                  : stacked
                    ? "grid grid-cols-[auto_minmax(0,1fr)] items-start gap-3"
                    : "grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-3"
            }`}
          >
            <div className="pointer-events-auto rounded-2xl border border-border bg-sheet shadow-float">
              {topLeft}
            </div>
            <div
              className={`pointer-events-auto w-full min-w-0 rounded-2xl border border-border bg-sheet shadow-float ${
                stacked ? "order-last col-span-2" : ""
              }`}
            >
              {top}
            </div>
            {topRight && !column ? (
              <div className="pointer-events-auto self-start justify-self-end rounded-2xl border border-border bg-sheet shadow-float">
                {topRight}
              </div>
            ) : null}
          </header>
          {editing ? (
            <div className="flex min-h-0 flex-1 gap-3">
              <div className="pointer-events-auto flex max-h-full self-start">{palette}</div>
              <div className="flex min-w-0 flex-1 flex-col items-center gap-3 pb-6">
                <div className="flex min-h-0 w-full flex-1 flex-col items-center">{toasts}</div>
                {dock}
                <div className="grid w-full grid-cols-[1fr_minmax(0,48rem)_1fr] items-end gap-3">
                  <span />
                  <div className="flex justify-center">{bottom}</div>
                  <div className="flex justify-end">{zoom}</div>
                </div>
              </div>
              {column ? null : (
                <div className="pointer-events-auto flex max-h-full self-start">{rail}</div>
              )}
            </div>
          ) : (
            <div className="min-w-0 flex-1 rounded-2xl border border-border bg-sheet shadow-float">
              {readonly}
            </div>
          )}
        </div>
        {column ? (
          <div className="flex shrink-0 flex-col items-end gap-3">
            {topRight ? (
              <div className="pointer-events-auto rounded-2xl border border-border bg-sheet shadow-float">
                {topRight}
              </div>
            ) : null}
            <div className="pointer-events-auto flex min-h-0 flex-1 items-start">{rail}</div>
          </div>
        ) : null}
      </div>
      {editing ? null : (
        <div className="pointer-events-none absolute inset-x-3 bottom-4 z-20 flex justify-center">
          {toasts}
        </div>
      )}
    </main>
  );
}
