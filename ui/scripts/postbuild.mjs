/* eslint-disable no-console -- a command-line script prints its result */
// After `vp build`: precompress, and put back `ui-dist/.gitkeep` (the build empties the folder so
// old hashed files never pile up in the binary). Usage: `node scripts/postbuild.mjs [dir]`.

import { existsSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { compressAssets } from "./compress.mjs";

const dir = resolve(process.argv[2] ?? "../crates/kaanal-server/ui-dist");
if (!existsSync(join(dir, ".vite", "manifest.json"))) {
  console.error(`postbuild: no manifest in ${dir}: the build did not run or its output moved`);
  process.exit(1);
}
console.log(`compressed ${compressAssets(dir)} assets`);
writeFileSync(join(dir, ".gitkeep"), "");
