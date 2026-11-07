// The canvas fills the whole window and the panels float over it, so "the view" a person sees is
// the canvas minus the space the panels take. These helpers say how much that is and what lands
// inside it. Pure, so they are tested without a browser.

import type { LayoutMode } from "../shell/layout/useLayout";

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
export function padsFor(
  mode: LayoutMode,
  railOpen = true,
  railW = 360,
  compactPalette = mode !== "full",
): Pads {
  return {
    top: mode === "full" ? 168 : 236,
    right: railOpen ? railW + 32 : 72,
    bottom: 96,
    left: compactPalette ? 92 : 328,
  };
}

/** The padding option of React Flow's `fitView`, in pixels. */
export function fitPadding(
  mode: LayoutMode,
  railOpen = true,
  railW = 360,
  compactPalette = mode !== "full",
): Record<keyof Pads, `${number}px`> {
  const p = padsFor(mode, railOpen, railW, compactPalette);
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

/**
 * The first free card slot in the visible area, reading left to right and then down, so a new card
 * lands where it can be seen and clicked and an old one is never pushed under a panel. Null when
 * every slot in view is taken (the caller falls back to `freeSpot`).
 */
export function freeSlotIn(
  region: { left: number; top: number; width: number; height: number },
  taken: [number, number][],
): [number, number] | null {
  const cols = Math.max(1, Math.floor(region.width / SLOT_W));
  const rows = Math.max(1, Math.floor(region.height / SLOT_H));
  const near = (p: [number, number]) =>
    taken.some((t) => Math.abs(t[0] - p[0]) < SLOT_W && Math.abs(t[1] - p[1]) < SLOT_H);
  const padX = (region.width - cols * SLOT_W) / 2;
  const padY = (region.height - rows * SLOT_H) / 2;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const p: [number, number] = [region.left + padX + c * SLOT_W, region.top + padY + r * SLOT_H];
      if (!near(p)) return p;
    }
  }
  return null;
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
