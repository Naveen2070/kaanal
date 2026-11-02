import { expect, test } from "vite-plus/test";
import { systemCounts } from "./counts";

test("parts_are_counted_by_sort_and_wires_by_edge", () => {
  const c = systemCounts({
    nodes: {
      gw: { type: "gateway" },
      a: { type: "service" },
      m: { type: "monolith" },
      db: { type: "database" },
      c: { type: "cache" },
      x: { type: "external-api" },
    },
    edges: [{ from: "gw", to: "a" }],
  });
  expect(c).toEqual({
    components: 6,
    traffic: 1,
    compute: 2,
    data: 2,
    external: 1,
    connections: 1,
  });
});

test("no_draft_counts_nothing", () => {
  expect(systemCounts(null).components).toBe(0);
  expect(systemCounts({}).connections).toBe(0);
});
