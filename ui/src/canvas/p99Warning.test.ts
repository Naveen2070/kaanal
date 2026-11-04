import { expect, test } from "vite-plus/test";
import { entryNode, p99Warning, p99WarningFor } from "./p99Warning";

const NO_CLIENT = "No client waits this long: add a queue limit or a timeout";

test("no warning at or under 30 s without an entry timeout", () => {
  expect(p99Warning(1_000, null)).toBeNull();
  expect(p99Warning(30_000, null)).toBeNull();
});

test("a p99 over 30 s warns that no client waits this long", () => {
  expect(p99Warning(30_001, null)).toBe(NO_CLIENT);
  expect(p99Warning(16_000, null)).toBeNull();
});

test("the entry timeout, when set, is the limit and is named", () => {
  expect(p99Warning(4_000, 5_000, "gateway")).toBeNull();
  const w = p99Warning(8_000, 5_000, "gateway");
  expect(w).toContain("longer than the timeout of gateway (5 s)");
  expect(w).toContain("8 s");
  // a long timeout does not hide a p99 the 30 s rule would not flag either
  expect(p99Warning(40_000, 60_000)).toBeNull();
});

test("nothing without a p99", () => {
  expect(p99Warning(null, null)).toBeNull();
  expect(p99Warning(undefined, 1_000)).toBeNull();
  expect(p99Warning(Number.NaN, null)).toBeNull();
});

test("the entry node is the gateway, else the root that owns the spec, else the first root", () => {
  const gw = {
    nodes: { app: { type: "service" }, gw: { type: "gateway", timeout: "500ms" } },
    edges: [{ from: "gw", to: "app" }],
  };
  expect(entryNode(gw)).toEqual({ name: "gw", timeoutMs: 500 });
  const none = {
    nodes: { db: { type: "database" }, app: { type: "service", spec: "o.yaml", timeout: "30s" } },
    edges: [],
  };
  expect(entryNode(none)).toEqual({ name: "app", timeoutMs: 30_000 });
  expect(
    entryNode({
      nodes: { a: { type: "service" }, b: { type: "service" } },
      edges: [{ from: "a", to: "b" }],
    }),
  ).toEqual({ name: "a", timeoutMs: null });
  expect(entryNode(undefined)).toBeNull();
});

test("p99WarningFor reads the timeout off the draft", () => {
  const doc = { nodes: { gw: { type: "gateway", timeout: "2s" } }, edges: [] };
  expect(p99WarningFor(3_000, doc)).toContain("timeout of gw (2 s)");
  expect(p99WarningFor(3_000, { nodes: { gw: { type: "gateway" } } })).toBeNull();
  expect(p99WarningFor(31_000, { nodes: { gw: { type: "gateway" } } })).toBe(NO_CLIENT);
});
