/* eslint-disable no-console -- a command-line script prints its result */
// Precompress the built assets (spec 11): a `.br` and a `.gz` next to every text asset, with Node's
// own zlib (no dependency). The server embeds them and picks one from Accept-Encoding. WOFF2 and
// PNG are already compressed and are left alone. Usage: `node scripts/compress.mjs [dir]`.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const TEXT = new Set([".js", ".css", ".svg"]);

/** Writes `<file>.br` and `<file>.gz` for each text file in `<dir>/assets`; returns the count. */
export function compressAssets(dir) {
  const assets = join(dir, "assets");
  let n = 0;
  for (const name of readdirSync(assets)) {
    if (!TEXT.has(extname(name))) continue;
    const bytes = readFileSync(join(assets, name));
    writeFileSync(
      join(assets, `${name}.br`),
      brotliCompressSync(bytes, {
        params: {
          [constants.BROTLI_PARAM_QUALITY]: 11,
          [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length,
        },
      }),
    );
    writeFileSync(join(assets, `${name}.gz`), gzipSync(bytes, { level: 9 }));
    n += 1;
  }
  return n;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dir = resolve(process.argv[2] ?? "../crates/kaanal-server/ui-dist");
  console.log(`compressed ${compressAssets(dir)} assets in ${dir}`);
}
