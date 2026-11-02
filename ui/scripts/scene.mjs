/* eslint-disable no-console -- a command-line script prints its result */
// Renders the loader's reflection images: the picture above the horizon (src/look/brand/Scene.ts) for each
// composition in fixed light and dark colours, into src/look/brand/scene/{wide,medium,narrow}-{light,dark}.svg.
// The loader shows one of them, flipped, in each of its 15 water bands. Run by hand after changing
// Scene.ts, geometry.mjs or the wordmark: `node scripts/scene.mjs` (Node strips the .ts types). The
// output is committed.

import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PALETTES, sceneFile } from "../src/look/brand/Scene.ts";
import { WORDMARK } from "../src/look/brand/wordmark.ts";

const dir = resolve(fileURLToPath(import.meta.url), "../../src/look/brand/scene");
mkdirSync(dir, { recursive: true });
for (const comp of ["wide", "medium", "narrow"]) {
  for (const theme of ["light", "dark"]) {
    const out = resolve(dir, `${comp}-${theme}.svg`);
    const svg = sceneFile(comp, PALETTES[theme], WORDMARK);
    writeFileSync(out, svg);
    console.log(`wrote ${out} (${svg.length} bytes)`);
  }
}
