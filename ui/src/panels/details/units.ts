// Units for the numbers a person types: the file keeps one base unit (milliseconds, kilobytes,
// kilobytes a second) and the box shows whichever unit reads best. A unit is a name and how many
// base units it holds. KB is 1024 bytes, as everywhere else in Studio.

export interface Unit {
  /** Shown in the option box. */
  label: string;
  /** Base units in one of this unit. */
  per: number;
}

export const TIME_UNITS: readonly Unit[] = [
  { label: "ms", per: 1 },
  { label: "s", per: 1000 },
  { label: "min", per: 60_000 },
];

export const SIZE_UNITS: readonly Unit[] = [
  { label: "KB", per: 1 },
  { label: "MB", per: 1024 },
  { label: "GB", per: 1024 * 1024 },
];

/** Body and header sizes, kept in whole bytes by the file (a header is a few hundred of them). */
export const BYTE_SIZE_UNITS: readonly Unit[] = [
  { label: "B", per: 1 },
  { label: "KB", per: 1024 },
  { label: "MB", per: 1024 ** 2 },
  { label: "GB", per: 1024 ** 3 },
];

export const RATE_UNITS: readonly Unit[] = [
  { label: "KB/s", per: 1 },
  { label: "MB/s", per: 1024 },
  { label: "GB/s", per: 1024 * 1024 },
];

const BYTE_UNITS = new Map([
  ["", 1],
  ["b", 1],
  ["kb", 1024],
  ["kib", 1024],
  ["mb", 1024 ** 2],
  ["mib", 1024 ** 2],
  ["gb", 1024 ** 3],
  ["gib", 1024 ** 3],
]);
const BYTES_TEXT = /^\s*(\d+(?:\.\d+)?)\s*([a-z]*)\s*(\/s)?\s*$/i;

/**
 * A byte or rate field of the draft as whole bytes (a rate: bytes a second), the way the loader
 * reads it (`units.rs`): a whole number, or text like `250KB`, `1.5 MB`, `512B`, `1MB/s`. KB is 1024
 * bytes, units are case-insensitive (`KiB` is the same), a fraction rounds to the nearest byte, and
 * `/s` is only for a `rate`. Anything else (negative, no number, an unknown unit, text that is not
 * a size) is null, so a field the page cannot read shows empty, never NaN.
 */
export function parseBytes(value: unknown, rate = false): number | null {
  if (typeof value === "number") return Number.isInteger(value) && value >= 0 ? value : null;
  if (typeof value !== "string") return null;
  const m = BYTES_TEXT.exec(value);
  if (!m || (m[3] && !rate)) return null;
  const per = BYTE_UNITS.get((m[2] ?? "").toLowerCase());
  if (per === undefined) return null;
  const bytes = Math.round(Number(m[1]) * per);
  return Number.isSafeInteger(bytes) ? bytes : null;
}

/** "280 KB/s", "1.5 MB/s": a rate in KB/s in the unit that reads best, three significant digits. */
export function rateText(kb: number): string {
  const u = [...RATE_UNITS].reverse().find((x) => kb >= x.per) ?? (RATE_UNITS[0] as Unit);
  return `${Number((kb / u.per).toPrecision(3)).toLocaleString("en-US")} ${u.label}`;
}

/** A float with the binary noise cut off: 0.1 * 3 gives 0.3, not 0.30000000000000004. */
const clean = (n: number): number => Number(n.toPrecision(12));

/** `base` base units, in `unit`. */
export const toUnit = (base: number, unit: Unit): number => clean(base / unit.per);

/** `base` in `unit` for a box: six significant digits, so 125 000 KB/s reads 0.119209 GB/s. */
export const display = (base: number, unit: Unit): number =>
  Number(toUnit(base, unit).toPrecision(6));

/** `shown` of `unit`, in base units. */
export const fromUnit = (shown: number, unit: Unit): number => clean(shown * unit.per);

/**
 * The largest unit in which `base` is still a whole number of at least 1 (125 000 KB/s is
 * 122.07... MB/s, not whole, so it stays KB/s; 131 072 KB/s is exactly 128 MB/s). Nothing to show
 * (null or zero) gives the smallest unit.
 */
export function pickUnit(units: readonly Unit[], base: number | null): Unit {
  const first = units[0] as Unit;
  if (base === null || !(base > 0)) return first;
  let best = first;
  for (const u of units) {
    const shown = base / u.per;
    if (shown >= 1 && Number.isInteger(clean(shown))) best = u;
  }
  return best;
}
