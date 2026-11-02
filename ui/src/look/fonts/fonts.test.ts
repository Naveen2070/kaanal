// The shipped fonts (spec 3): every file exists, FONTS.md names it with an OFL licence, the text face
// has tabular figures (a `tnum` feature in its GSUB table, read straight from the WOFF2), and the
// Tamil subset really is a subset.

import { existsSync, readFileSync, statSync } from "node:fs";
import { brotliDecompressSync } from "node:zlib";
import { describe, expect, it } from "vite-plus/test";

// vitest runs from ui/ (jsdom gives import.meta.url a non-file scheme)
const dir = "src/look/fonts/";
const FILES = ["inter-latin.woff2", "jetbrains-mono-latin.woff2", "noto-sans-tamil-lockup.woff2"];
const fontsMd = readFileSync("FONTS.md", "utf8");

/** WOFF2 known-table tags by directory flag (W3C WOFF2 spec, section 5.1). */
const KNOWN = [
  "cmap",
  "head",
  "hhea",
  "hmtx",
  "maxp",
  "name",
  "OS/2",
  "post",
  "cvt ",
  "fpgm",
  "glyf",
  "loca",
  "prep",
  "CFF ",
  "VORG",
  "EBDT",
  "EBLC",
  "gasp",
  "hdmx",
  "kern",
  "LTSH",
  "PCLT",
  "VDMX",
  "vhea",
  "vmtx",
  "BASE",
  "GDEF",
  "GPOS",
  "GSUB",
];

/** The GSUB feature tags of a WOFF2 file, or [] when it has no GSUB table. */
function gsubFeatures(file: string): string[] {
  const b = readFileSync(file);
  expect(b.toString("latin1", 0, 4)).toBe("wOF2");
  const numTables = b.readUInt16BE(12);
  let p = 48;
  const base128 = () => {
    let v = 0;
    for (let i = 0; i < 5; i++) {
      const byte = b[p++]!;
      v = v * 128 + (byte & 0x7f);
      if (!(byte & 0x80)) return v;
    }
    throw new Error("bad UIntBase128");
  };
  const tables: { tag: string; size: number }[] = [];
  for (let i = 0; i < numTables; i++) {
    const flags = b[p++]!;
    const idx = flags & 0x3f;
    const tag = idx === 63 ? b.toString("latin1", p, (p += 4)) : (KNOWN[idx] ?? `#${idx}`);
    const version = flags >> 6;
    const orig = base128();
    const transformed = tag === "glyf" || tag === "loca" ? version === 0 : version !== 0;
    tables.push({ tag, size: transformed ? base128() : orig });
  }
  const totalCompressed = b.readUInt32BE(20);
  const data = brotliDecompressSync(b.subarray(p, p + totalCompressed));
  let off = 0;
  for (const t of tables) {
    if (t.tag === "GSUB") {
      const list = off + data.readUInt16BE(off + 6);
      const n = data.readUInt16BE(list);
      return Array.from({ length: n }, (_, i) =>
        data.toString("latin1", list + 2 + i * 6, list + 6 + i * 6),
      );
    }
    off += t.size;
  }
  return [];
}

describe("fonts", () => {
  it("fonts_have_tabular_figures_and_licences_listed", () => {
    for (const f of FILES) {
      expect(existsSync(dir + f), f).toBe(true);
      const line = fontsMd.split("\n").find((l) => l.includes(f));
      expect(line, `${f} is listed in FONTS.md`).toBeDefined();
      expect(line, `${f} has an OFL licence`).toMatch(/OFL/);
    }
    expect(gsubFeatures(dir + "inter-latin.woff2")).toContain("tnum");
  });

  it("only the Latin text face is a first-load font, inside the budget", () => {
    expect(statSync(dir + "inter-latin.woff2").size).toBeLessThan(60 * 1024);
  });

  it("the Tamil face is the lockup subset, not the whole script", () => {
    expect(statSync(dir + "noto-sans-tamil-lockup.woff2").size).toBeLessThan(8 * 1024);
  });
});
