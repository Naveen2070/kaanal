// The Log and Results tabs are separate chunks (spec 11): the panel reaches them only through the
// `.lazy` modules, so the first load does not carry them. The built manifest is checked by `pnpm run size`.
import { readFileSync } from "node:fs";
import { expect, test } from "vite-plus/test";

const read = (f: string) => readFileSync(new URL(f, import.meta.url), "utf8");

test("tab_is_a_separate_chunk", () => {
  const panel = read("./Panel.tsx");
  expect(panel).not.toMatch(/from "\.\/(LogTab|Results)"/);
  expect(read("./LogTab.lazy.ts")).toMatch(/import\("\.\/LogTab"\)/);
  expect(read("./Results.lazy.ts")).toMatch(/import\("\.\/Results"\)/);
});

test("charts_band_is_a_lazy_chunk", () => {
  expect(read("../App.tsx")).not.toMatch(/from "\.\/charts\/ChartsBand"/);
  expect(read("../charts/ChartsBand.lazy.ts")).toMatch(/import\("\.\/ChartsBand"\)/);
});

test("tour_is_a_lazy_chunk", () => {
  expect(read("../first-run/FirstRun.tsx")).not.toMatch(/from "\.\/Tour"/);
  expect(read("../first-run/FirstRun.tsx")).toMatch(/import\("\.\/Tour"\)/);
});
