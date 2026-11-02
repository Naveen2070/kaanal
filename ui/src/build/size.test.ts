// The size budget (spec 11): first-load JS + CSS gzip and first-load fonts, read from the Vite manifest.

import { randomBytes } from "node:crypto";
import { appendFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vite-plus/test";
import { check } from "../../scripts/size.mjs";

const dirs: string[] = [];
afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

/** A fake ui-dist whose entry loads `js` bytes of incompressible JS, `css` of CSS and one font. */
function bundle(js: number, fontBytes: number, lazyJs = 5000): string {
  const dir = mkdtempSync(join(tmpdir(), "kaanal-size-"));
  dirs.push(dir);
  mkdirSync(join(dir, "assets"));
  mkdirSync(join(dir, ".vite"));
  writeFileSync(join(dir, "assets", "index-a1.js"), randomBytes(js));
  writeFileSync(join(dir, "assets", "index-a1.css"), randomBytes(200));
  writeFileSync(join(dir, "assets", "inter-latin-b2.woff2"), randomBytes(fontBytes));
  writeFileSync(join(dir, "assets", "log-tab-c3.js"), randomBytes(lazyJs));
  writeFileSync(
    join(dir, ".vite", "manifest.json"),
    JSON.stringify({
      "index.html": {
        file: "assets/index-a1.js",
        isEntry: true,
        assets: ["assets/inter-latin-b2.woff2"],
        dynamicImports: ["src/panels/details/LogTab.tsx"],
      },
      "style.css": { file: "assets/index-a1.css", src: "style.css" },
      "src/panels/details/LogTab.tsx": { file: "assets/log-tab-c3.js", isDynamicEntry: true },
    }),
  );
  return dir;
}

describe("size budget", () => {
  const budget = { jsCssGzipKb: 1, fontsKb: 0.5, firstLoadFonts: ["inter"] };

  it("passes under budget and counts only first-load files (lazy chunks are free)", () => {
    const r = check(bundle(400, 300, 50_000), budget);
    expect(r.ok).toBe(true);
    expect(r.problems).toEqual([]);
  });

  it("fails when first-load JS + CSS gzip is over budget", () => {
    const r = check(bundle(2000, 300), budget);
    expect(r.ok).toBe(false);
    expect(r.problems.join(" ")).toMatch(/JS \+ CSS/);
  });

  it("counts the stylesheet that sits under its own manifest key", () => {
    const r = check(bundle(100, 300), { ...budget, jsCssGzipKb: 0.2 });
    expect(r.ok).toBe(false);
    expect(r.jsCssGzipKb).toBeGreaterThan(0.25);
  });

  it("fails when first-load fonts are over budget", () => {
    const r = check(bundle(400, 700), budget);
    expect(r.ok).toBe(false);
    expect(r.problems.join(" ")).toMatch(/fonts/);
  });

  it("fails when a source map or a sourceMappingURL ships", () => {
    const withMap = bundle(400, 300);
    writeFileSync(join(withMap, "assets", "index-a1.js.map"), "{}");
    expect(check(withMap, budget).problems.join(" ")).toMatch(/source map/);

    const withComment = bundle(400, 300);
    appendFileSync(join(withComment, "assets", "index-a1.js"), "\n//# sourceMappingURL=x.map");
    expect(check(withComment, budget).problems.join(" ")).toMatch(/sourceMappingURL/);
  });

  it("fails clearly when the manifest is missing", () => {
    const dir = mkdtempSync(join(tmpdir(), "kaanal-size-"));
    dirs.push(dir);
    const r = check(dir, budget);
    expect(r.ok).toBe(false);
    expect(r.problems.join(" ")).toMatch(/manifest/);
  });
});
