// The control bar's log sliders (spec 4): Time runs 0.1 ms to 60 s and Slots 1 to 100,000, both over
// 1,000 integer positions, so a thumb reaches 1 ms and 10 s alike. The ends are exact; a value in
// between comes back from its position within 1% for Time (half a 1.3% step, three decimals) and within
// one 1.2% step for Slots (whole numbers).
// A typed value outside the range pins the thumb to the nearer end (0 ms sits at 0.1 ms).

export const LOG_STEPS = 1000;

const clamp = (x: number, lo: number, hi: number) =>
  Number.isFinite(x) ? Math.min(Math.max(x, lo), hi) : lo;

const at = (lo: number, hi: number, p: number) =>
  lo * Math.pow(hi / lo, clamp(p, 0, LOG_STEPS) / LOG_STEPS);

const position = (lo: number, hi: number, v: number) =>
  Math.round((LOG_STEPS * Math.log(clamp(v, lo, hi) / lo)) / Math.log(hi / lo));

const MS_LO = 0.1;
const MS_HI = 60_000;

/**
 * Milliseconds at position `p` (0 to 1000): four significant digits and never past three decimals
 * (0.1, 0.101, 25.27, 60000), because the draft keeps a duration to a microsecond: the number the
 * bar shows is the number it writes.
 */
export function msFromSlider(p: number): number {
  const ms = Number(at(MS_LO, MS_HI, p).toPrecision(4));
  return clamp(Math.round(ms * 1000) / 1000, MS_LO, MS_HI);
}

export function sliderFromMs(ms: number): number {
  return position(MS_LO, MS_HI, ms);
}

const SLOTS_LO = 1;
const SLOTS_HI = 100_000;

/** Whole slots at position `p` (0 to 1000). */
export function slotsFromSlider(p: number): number {
  return Math.round(at(SLOTS_LO, SLOTS_HI, p));
}

export function sliderFromSlots(n: number): number {
  return position(SLOTS_LO, SLOTS_HI, n);
}
