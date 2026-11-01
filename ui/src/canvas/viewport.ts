// The canvas fills the whole window and the panels float over it, so "the view" a person sees is
// the canvas minus the space the panels take. These helpers say how much that is and what lands
// inside it. Pure, so they are tested without a browser.

import type { LayoutMode } from "../layout/useLayout";

export interface Pads {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * Space the floating panels take: the top bar and key figures above, the palette on the left (an
 * icon rail in the middle layout), the details rail on the right (a slim handle when folded), and room for the dock below.
 */
export function padsFor(mode: LayoutMode, railOpen = true): Pads {
  return { top: 168, right: railOpen ? 392 : 72, bottom: 96, left: mode === "full" ? 328 : 84 };
}

/** The padding option of React Flow's `fitView`, in pixels. */
export function fitPadding(mode: LayoutMode, railOpen = true): Record<keyof Pads, `${number}px`> {
  const p = padsFor(mode, railOpen);
  return {
    top: `${p.top}px`,
    right: `${p.right}px`,
    bottom: `${p.bottom}px`,
    left: `${p.left}px`,
  };
}

/** The middle of what is visible between the panels, in screen coordinates. */
export function visibleCentre(canvas: Box, pads: Pads): { x: number; y: number } {
  return {
    x: (canvas.left + pads.left + canvas.right - pads.right) / 2,
    y: (canvas.top + pads.top + canvas.bottom - pads.bottom) / 2,
  };
}

/** A card's size in flow units plus a gap, for keeping new cards off old ones. */
const SLOT_W = 260;
const SLOT_H = 210;

/** `at`, or the nearest free slot to the right of it, when a card already sits there. */
export function freeSpot(at: [number, number], taken: [number, number][]): [number, number] {
  const hit = (p: [number, number]) =>
    taken.some((t) => Math.abs(t[0] - p[0]) < SLOT_W && Math.abs(t[1] - p[1]) < SLOT_H);
  let p = at;
  for (let n = 0; n < 50 && hit(p); n++) {
    p = [p[0] + SLOT_W, p[1]];
    if (n % 6 === 5) p = [at[0], p[1] + SLOT_H]; // a row full: the next row
  }
  return p;
}

/** True when `box` (screen coordinates) lies wholly in the visible area. */
export function isVisible(box: Box, canvas: Box, pads: Pads): boolean {
  return (
    box.left >= canvas.left + pads.left &&
    box.right <= canvas.right - pads.right &&
    box.top >= canvas.top + pads.top &&
    box.bottom <= canvas.bottom - pads.bottom
  );
}
