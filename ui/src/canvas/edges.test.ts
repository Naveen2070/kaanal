import { expect, test } from "vite-plus/test";
import type { Doc } from "../api/types";
import { autoLayout, connectOps, decodeId, encodeId, lastEdgeIndex, uniqueName } from "./edges";

const doc: Doc = {
  nodes: { gw: { type: "gateway" }, app: { type: "service" }, db: { type: "database" } },
  edges: [
    { from: "gw", to: "app" },
    { from: "app", to: "db" },
  ],
};

test("names_with_hash_slash_space_and_unicode_survive_a_round_trip", () => {
  for (const name of ["a#b", "a/b", "a b", "é日本", "100%", "a?b=c&d", "", "..", "x%2Fy"]) {
    expect(decodeId(encodeId(name))).toBe(name);
  }
  // an id never contains a character that is special in a URL or a CSS selector
  expect(encodeId("a#b/c d")).not.toMatch(/[#/ ]/);
});

test("a_new_name_is_unique_among_the_taken_ones", () => {
  expect(uniqueName("service", [])).toBe("service");
  expect(uniqueName("service", ["service"])).toBe("service-2");
  expect(uniqueName("service", ["service", "service-2", "service-3"])).toBe("service-4");
  expect(uniqueName("service", ["service-2"])).toBe("service");
});

test("connecting_appends_to_a_list_and_otherwise_writes_the_key", () => {
  expect(connectOps(doc, "gw", "db")).toEqual([
    { op: "append", path: ["edges"], value: { from: "gw", to: "db" } },
  ]);
  expect(connectOps({ nodes: {} }, "a", "b")).toEqual([
    { op: "insert", path: ["edges"], value: [{ from: "a", to: "b" }] },
  ]);
  // `edges:` with no value is a key that exists
  expect(connectOps({ nodes: {}, edges: null as never }, "a", "b")).toEqual([
    { op: "set", path: ["edges"], value: [{ from: "a", to: "b" }] },
  ]);
});

test("connecting_gateway_to_service_wires_unchained_endpoints", () => {
  const d: Doc = {
    nodes: { gw: { type: "gateway" }, app: { type: "service" } },
    edges: [],
    endpoints: {
      "GET /": { override: { body: { ok: true } } },
    },
  };
  expect(connectOps(d, "gw", "app")).toEqual([
    { op: "append", path: ["edges"], value: { from: "gw", to: "app" } },
    { op: "insert", path: ["endpoints", "GET /", "chain"], value: [{ call: "app" }] },
  ]);
});

test("the_last_matching_edge_is_found_even_when_an_end_is_a_number", () => {
  const d: Doc = {
    edges: [
      { from: 1, to: 2 },
      { from: "a", to: "b" },
      { from: 1, to: 2 },
    ],
  };
  expect(lastEdgeIndex(d, "1", "2")).toBe(2);
  expect(lastEdgeIndex(d, "a", "b")).toBe(1);
  expect(lastEdgeIndex(d, "b", "a")).toBe(-1);
});

test("nodes_without_a_layout_get_a_left_to_right_layout", () => {
  const l = autoLayout(doc);
  const [gw, app, db] = [l.gw, l.app, l.db];
  expect(gw?.[0]).toBeLessThan(app?.[0] ?? 0);
  expect(app?.[0]).toBeLessThan(db?.[0] ?? 0);
  // two nodes at one depth do not overlap
  const two = autoLayout({
    nodes: { gw: { type: "gateway" }, a: { type: "service" }, b: { type: "service" } },
    edges: [
      { from: "gw", to: "a" },
      { from: "gw", to: "b" },
    ],
  });
  expect(two.a?.[0]).toBe(two.b?.[0]);
  expect(two.a?.[1]).not.toBe(two.b?.[1]);
});

test("a_cycle_and_a_loose_node_are_still_placed", () => {
  const l = autoLayout({
    nodes: { a: { type: "service" }, b: { type: "service" }, c: { type: "cache" } },
    edges: [
      { from: "a", to: "b" },
      { from: "b", to: "a" },
    ],
  });
  expect(Object.keys(l).sort()).toEqual(["a", "b", "c"]);
});
