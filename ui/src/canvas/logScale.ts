// The offered-load slider is logarithmic: 10 to 15,000 requests per second over 1,000 steps, so
// 10 and 10,000 are both within reach of a thumb. A typed value outside the range pins the thumb
// to the nearer end; the figure shown is always the integer that is sent.

const LO = 10;
const HI = 15000;
export const SLIDER_MAX = 1000;

const clamp = (x: number, lo: number, hi: number) =>
  Number.isFinite(x) ? Math.min(Math.max(x, lo), hi) : lo;

/** The rate at slider position `v` (0 to 1000). */
export function rpsFromSlider(v: number): number {
  return Math.round(LO * Math.pow(HI / LO, clamp(v, 0, SLIDER_MAX) / SLIDER_MAX));
}

/** The slider position (an integer, 0 to 1000) of `rps`. */
export function sliderFromRps(rps: number): number {
  return Math.round((SLIDER_MAX * Math.log(clamp(rps, LO, HI) / LO)) / Math.log(HI / LO));
}
