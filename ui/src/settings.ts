// Settings that outlive a page load, kept in localStorage (a settings menu will call the setters).
// Storage can be blocked (a private window): a read then gives the default, a write is dropped.

const MIN_LOAD_KEY = "kaanal-min-load-ms";
/** The loader stays at least this long, so it is seen rather than flashed (0 turns it off). */
export const DEFAULT_MIN_LOAD_MS = 10_000;
const MAX_MIN_LOAD_MS = 60_000;

export function getMinLoadMs(): number {
  try {
    const v = localStorage.getItem(MIN_LOAD_KEY);
    if (v !== null) {
      const n = Number(v);
      if (Number.isFinite(n) && n >= 0) return Math.min(n, MAX_MIN_LOAD_MS);
    }
  } catch {
    // blocked storage: the default
  }
  return DEFAULT_MIN_LOAD_MS;
}

export function setMinLoadMs(ms: number): void {
  try {
    localStorage.setItem(MIN_LOAD_KEY, String(Math.min(Math.max(ms, 0), MAX_MIN_LOAD_MS)));
  } catch {
    // blocked storage: this page keeps the old value
  }
}
