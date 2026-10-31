/* eslint-disable no-console -- a command-line script prints its result */
// The first-load size budget (spec 11). Reads the Vite manifest, follows the entry's static imports
// and counts what a first load fetches: JS + CSS as gzip, fonts raw (WOFF2 does not gzip). Lazy
// chunks (dynamic imports) are not counted. Usage: `node scripts/size.mjs [dir]` (default
// ../crates/kaanal-server/ui-dist); also fails on any source map or `sourceMappingURL`; exits 1 on a problem.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

/** Guesses to revisit once measured (today 139 kB gzip with React Flow). */
export const BUDGET = { jsCssGzipKb: 200, fontsKb: 60, firstLoadFonts: ["inter"] };

const kb = (n) => n / 1024;

/** @returns {{ ok: boolean, jsCssGzipKb: number, fontsKb: number, problems: string[] }} */
export function check(dir, budget = BUDGET) {
  const manifestPath = join(dir, ".vite", "manifest.json");
  if (!existsSync(manifestPath)) {
    return {
      ok: false,
      jsCssGzipKb: 0,
      fontsKb: 0,
      problems: [`no Vite manifest at ${manifestPath}: run \`pnpm run build\` first`],
    };
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const entries = Object.entries(manifest).filter(([, v]) => v.isEntry);
  const seen = new Set();
  const files = new Set();
  const visit = (key) => {
    if (seen.has(key) || !manifest[key]) return;
    seen.add(key);
    const c = manifest[key];
    files.add(c.file);
    for (const f of c.css ?? []) files.add(f);
    for (const f of c.assets ?? []) files.add(f);
    for (const k of c.imports ?? []) visit(k);
  };
  for (const [key] of entries) visit(key);
  // with `cssCodeSplit: false` the one stylesheet is its own manifest key, not under the entry
  for (const key of Object.keys(manifest)) if (key.endsWith(".css")) visit(key);

  let jsCss = 0;
  let fonts = 0;
  for (const f of files) {
    const path = join(dir, f);
    if (!existsSync(path)) continue;
    const bytes = readFileSync(path);
    if (f.endsWith(".woff2")) {
      if (budget.firstLoadFonts.some((w) => f.toLowerCase().includes(w))) fonts += bytes.length;
    } else if (f.endsWith(".js") || f.endsWith(".css")) {
      jsCss += gzipSync(bytes, { level: 9 }).length;
    }
  }
  const report = { ok: true, jsCssGzipKb: kb(jsCss), fontsKb: kb(fonts), problems: [] };
  // spec 11: a production bundle carries no source map and no pointer to one
  for (const f of readdirSync(dir, { recursive: true, encoding: "utf8" })) {
    const name = f.replaceAll("\\", "/");
    if (name.endsWith(".map")) report.problems.push(`source map in the bundle: ${name}`);
    else if (
      /\.(js|css)$/.test(name) &&
      readFileSync(join(dir, f), "utf8").includes("sourceMappingURL=")
    ) {
      report.problems.push(`sourceMappingURL in the bundle: ${name}`);
    }
  }
  if (report.jsCssGzipKb > budget.jsCssGzipKb) {
    report.problems.push(
      `first-load JS + CSS is ${report.jsCssGzipKb.toFixed(1)} kB gzip, budget ${budget.jsCssGzipKb} kB`,
    );
  }
  if (report.fontsKb > budget.fontsKb) {
    report.problems.push(
      `first-load fonts are ${report.fontsKb.toFixed(1)} kB, budget ${budget.fontsKb} kB`,
    );
  }
  report.ok = report.problems.length === 0;
  return report;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dir = resolve(process.argv[2] ?? "../crates/kaanal-server/ui-dist");
  const r = check(dir);
  console.log(
    `first load: JS + CSS ${r.jsCssGzipKb.toFixed(1)} kB gzip (budget ${BUDGET.jsCssGzipKb}), fonts ${r.fontsKb.toFixed(1)} kB (budget ${BUDGET.fontsKb})`,
  );
  for (const p of r.problems) console.error(`FAIL: ${p}`);
  process.exit(r.ok ? 0 : 1);
}
