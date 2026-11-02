export interface Disc {
  cx: number;
  cy: number;
  r: number;
}
export interface Bar {
  y: number;
  h: number;
}
export interface Slice {
  x: number;
  y: number;
  w: number;
  h: number;
  opacity: number;
}

export const GRID: number;
export const DISC: Disc;
export const SUN_BARS: Bar[];
export const HORIZON: { x1: number; x2: number; y: number; width: number; opacity: number };
export const SLICES: Slice[];
export const FAVICON: { grid: number; disc: Disc; bars: Bar[]; slice: Slice };
export const TILE: string;
export function discPath(d: Disc): string;
