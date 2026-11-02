// The Kaanal mark's numbers (spec 12, "the mirage sun"), in one place: Mark.tsx draws them and
// scripts/icons.mjs rasterises them, so the two cannot drift. A plain .mjs so Node can import it.

/** 24 grid: a half disc on a horizon, cut into bars by heat-haze gaps that grow toward the horizon. */
export const GRID = 24;
export const DISC = { cx: 12, cy: 13.5, r: 8.8 };

/** Top to bottom. Gaps between them: 0.9, then 1.5. Bars are clipped to the disc, never beyond it. */
export const SUN_BARS = [
  { y: 3.9, h: 3.7 },
  { y: 8.5, h: 2.4 },
  { y: 12.4, h: 1.1 },
];

export const HORIZON = { x1: 1, x2: 23, y: 14.9, width: 1.1, opacity: 0.55 };

/** The reflection: three slices that shear sideways (left, right, left) and fade. */
export const SLICES = [
  { x: 4.6, y: 16.3, w: 14.8, h: 1.8, opacity: 0.7 },
  { x: 7.4, y: 19, w: 11.2, h: 1.6, opacity: 0.45 },
  { x: 3.2, y: 21.5, w: 7.6, h: 1.3, opacity: 0.26 },
];

/** A separate 16 grid master: two bars, one 1 px gap, one reflection slice, no horizon line. */
export const FAVICON = {
  grid: 16,
  disc: { cx: 8, cy: 9, r: 6 },
  bars: [
    { y: 3, h: 3 },
    { y: 7, h: 2 },
  ],
  slice: { x: 4, y: 10.5, w: 8, h: 2.5, opacity: 0.7 },
};

/** The brand blue of the app tile: a constant, the same in both themes. */
export const TILE = "#1f6fe0";

/** The half disc as an SVG path. */
export function discPath({ cx, cy, r }) {
  return `M${cx - r} ${cy}A${r} ${r} 0 0 1 ${cx + r} ${cy}Z`;
}
