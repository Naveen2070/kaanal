// Settings that outlive a page load, kept in localStorage (a settings menu will call the setters).
// Storage can be blocked (a private window): a read then gives the default, a write is dropped.

const MIN_LOAD_KEY = "kaanal-min-load-ms";
/** The loader stays at least this long, so it is seen rather than flashed (0 turns it off). */
export const DEFAULT_MIN_LOAD_MS = 2_500;
/** From the second visit on, the full hold is only a wait: it drops to this unless a setting says otherwise. */
export const REPEAT_MIN_LOAD_MS = 2_500;
const VISITED_KEY = "kaanal-visited";
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
  try {
    if (localStorage.getItem(VISITED_KEY) !== null) return REPEAT_MIN_LOAD_MS;
  } catch {
    // blocked storage: the full hold
  }
  return DEFAULT_MIN_LOAD_MS;
}

/** The loader has been seen once: later loads hold for `REPEAT_MIN_LOAD_MS`. */
export function markVisited(): void {
  try {
    localStorage.setItem(VISITED_KEY, "1");
  } catch {
    // blocked storage: the full hold stays
  }
}

export function setMinLoadMs(ms: number): void {
  try {
    localStorage.setItem(MIN_LOAD_KEY, String(Math.min(Math.max(ms, 0), MAX_MIN_LOAD_MS)));
  } catch {
    // blocked storage: this page keeps the old value
  }
}

const GROUPS_KEY = "kaanal-groups-open";

/** Which Controls groups the person left open or closed, by group id (a missing id = its default). */
export function getGroupsOpen(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(GROUPS_KEY);
    if (raw === null) return {};
    const v: unknown = JSON.parse(raw);
    if (v === null || typeof v !== "object" || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter(([, o]) => typeof o === "boolean"));
  } catch {
    return {}; // blocked storage or junk: every group at its default
  }
}

export function setGroupsOpen(open: Record<string, boolean>): void {
  try {
    localStorage.setItem(GROUPS_KEY, JSON.stringify(open));
  } catch {
    // blocked storage: this page keeps them, a reload forgets
  }
}
