// WCAG contrast of the design tokens (spec 3): text 4.5:1, non-text 3:1, in both themes. The values
// are read from tokens.css itself, so a changed colour is checked without touching this file.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vite-plus/test";

// vitest runs from ui/ (jsdom gives import.meta.url a non-file scheme)
const css = readFileSync("src/theme/tokens.css", "utf8");

type Pair = [light: string, dark: string];

/** `--name: light-dark(#a, #b)` or a plain `--name: #a` (same in both themes). */
function tokens(): Map<string, Pair> {
  const out = new Map<string, Pair>();
  for (const m of css.matchAll(
    /--([a-z-]+):\s*(?:light-dark\(\s*(#\w+)\s*,\s*(#\w+)\s*\)|(#\w+))\s*;/g,
  )) {
    out.set(m[1]!, [(m[2] ?? m[4])!, (m[3] ?? m[4])!]);
  }
  return out;
}

function channel(v: number): number {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string): number {
  let h = hex.slice(1);
  if (h.length === 3) h = h.replace(/./g, (c) => c + c);
  const n = Number.parseInt(h, 16);
  return (
    0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255)
  );
}

export function ratio(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const T = tokens();
const get = (name: string, theme: 0 | 1): string => {
  const v = T.get(name);
  if (!v) throw new Error(`token --${name} is missing from tokens.css`);
  return v[theme];
};

/** [foreground, background, minimum ratio] */
const PAIRS: [string, string, number][] = [
  ...["ground", "sheet", "well", "raised", "active"].flatMap((bg): [string, string, number][] => [
    ["text", bg, 4.5],
    ["muted", bg, 4.5],
  ]),
  ["faint", "sheet", 4.5],
  ["faint", "well", 4.5],
  ["accent", "sheet", 4.5],
  ["accent", "well", 3],
  ["positive", "sheet", 4.5],
  ["warn", "sheet", 4.5],
  ["negative", "sheet", 4.5],
  ["control", "sheet", 3],
  ["ok-fill", "well", 3],
  ["warn-fill", "well", 3],
  ["bad-fill", "well", 3],
  ["on-primary", "primary", 4.5],
  ["on-tip", "tip", 4.5],
];

describe("design tokens", () => {
  it("contrast_tokens_meet_wcag", () => {
    const failures: string[] = [];
    for (const [i, theme] of ["light", "dark"].entries()) {
      for (const [fg, bg, min] of PAIRS) {
        const r = ratio(get(fg, i as 0 | 1), get(bg, i as 0 | 1));
        if (r < min)
          failures.push(`${theme}: --${fg} on --${bg} is ${r.toFixed(2)}:1, needs ${min}`);
      }
    }
    expect(failures).toEqual([]);
  });

  it("defines every token the plan promises, for both themes", () => {
    for (const name of [
      "ground",
      "sheet",
      "well",
      "raised",
      "active",
      "border",
      "rule",
      "control",
      "text",
      "muted",
      "faint",
      "accent",
      "positive",
      "warn",
      "negative",
      "ok-fill",
      "warn-fill",
      "bad-fill",
    ]) {
      expect(T.has(name), `--${name}`).toBe(true);
    }
  });

  it("uses the spec's surfaces and control borders", () => {
    expect([get("ground", 0), get("sheet", 0), get("well", 0)]).toEqual([
      "#f3f6fa",
      "#ffffff",
      "#f4f6f8",
    ]);
    expect([get("ground", 1), get("sheet", 1), get("well", 1)]).toEqual([
      "#000000",
      "#0e0e0e",
      "#161616",
    ]);
    expect([get("control", 0), get("control", 1)]).toEqual(["#8a93a3", "#6b6b6b"]);
    expect([get("accent", 0), get("accent", 1)]).toEqual(["#1f6fe0", "#5aa2ff"]);
  });
});
