import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { mockFetch } from "../../test/fetch";
import { useDraft } from "./draft";

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b1",
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service" }, db: { type: "database" } },
      edges: [{ from: "gw", to: "app" }],
    },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: { service: { type: "service", concurrency: 32 } },
    layout: { gw: [0, 0], app: [280, 0], db: [560, 0] },
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...over,
  };
}

async function loaded(first: DraftView) {
  mockFetch([{ json: first }]);
  await useDraft.getState().load();
}

const withEdge = (to: string) =>
  view({
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service" }, db: { type: "database" } },
      edges: [
        { from: "gw", to: "app" },
        { from: "app", to },
      ],
    },
  });

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => vi.unstubAllGlobals());

test("dragging_a_palette_item_inserts_a_node_with_a_unique_name", async () => {
  await loaded(view());
  const doc = (n: string) => ({
    nodes: {
      gw: { type: "gateway" },
      app: { type: "service" },
      db: { type: "database" },
      [n]: { type: "service" },
    },
    edges: [],
  });
  const sent = mockFetch([{ json: view({ doc: doc("service"), layout: { service: [10, 20] } }) }]);
  expect(await useDraft.getState().addNode("service", [10, 20])).toBe("service");
  // the node and its spot are one request, so a node never shows without a position
  expect(sent).toHaveLength(1);
  expect(sent[0]?.body).toMatchObject({
    patch: [{ op: "add_node", name: "service", kind: "service" }],
    layout: { service: [10, 20], gw: [0, 0] },
  });
  // a second one of the same kind is named apart
  const second = { ...doc("service") };
  second.nodes = { ...second.nodes, "service-2": { type: "service" } };
  const sent2 = mockFetch([{ json: view({ doc: doc("service") }) }]);
  expect(await useDraft.getState().addNode("service")).toBe("service-2");
  expect(sent2[0]?.body).toMatchObject({
    patch: [{ op: "add_node", name: "service-2", kind: "service" }],
  });
});

test("a_click_without_a_drop_point_places_the_node_right_of_the_others", async () => {
  await loaded(view());
  const sent = mockFetch([{ json: view() }]);
  await useDraft.getState().addNode("cache");
  expect(sent[0]?.body).toMatchObject({ layout: { cache: [840, 0] } });
});

test("connecting_two_nodes_patches_an_edge_and_validates", async () => {
  await loaded(view());
  const sent = mockFetch([
    { json: withEdge("db") },
    { json: { ok: true, diagnostics: [], warnings: [], flagged: [] } },
  ]);
  expect(await useDraft.getState().connect("app", "db")).toEqual({ ok: true });
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "append", path: ["edges"], value: { from: "app", to: "db" } }],
  });
  expect(sent[1]?.url).toBe("/__kaanal/draft/validate");
});

test("a_refused_connection_is_rolled_back_with_the_message", async () => {
  await loaded(view());
  const bad = {
    ok: false,
    diagnostics: [
      {
        file: null,
        line: 9,
        col: 2,
        path: ["edges", 1, "to"],
        message: "a node cannot call itself",
      },
    ],
    warnings: [],
    flagged: [],
  };
  const sent = mockFetch([{ json: withEdge("app") }, { json: bad }, { json: view() }]);
  const r = await useDraft.getState().connect("app", "app");
  expect(r).toEqual({ ok: false, message: "a node cannot call itself" });
  expect(sent[2]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 1] }] });
  expect(useDraft.getState().view?.doc?.edges).toHaveLength(1);
});

test("a_problem_elsewhere_in_the_draft_keeps_the_edge_and_warns", async () => {
  await loaded(view());
  const elsewhere = {
    ok: false,
    diagnostics: [{ file: null, line: 1, col: 0, path: ["nodes"], message: "no entry node" }],
    warnings: [],
    flagged: [],
  };
  mockFetch([{ json: withEdge("db") }, { json: elsewhere }]);
  expect(await useDraft.getState().connect("app", "db")).toEqual({
    ok: true,
    warning: "no entry node",
  });
  expect(useDraft.getState().view?.doc?.edges).toHaveLength(2);
});

test("an_edge_the_patch_refuses_comes_back_as_a_message", async () => {
  await loaded(view());
  mockFetch([{ status: 400, json: { error: "nothing at edges" } }]);
  expect(await useDraft.getState().connect("app", "db")).toEqual({
    ok: false,
    message: "nothing at edges",
  });
});

test("a_problem_in_another_edge_does_not_take_this_edge_back", async () => {
  await loaded(view());
  const other = {
    ok: false,
    diagnostics: [
      { file: null, line: 9, col: 2, path: ["edges", 0, "from"], message: "no node 'zz'" },
    ],
    warnings: [],
    flagged: [],
  };
  const sent = mockFetch([{ json: withEdge("db") }, { json: other }]);
  expect(await useDraft.getState().connect("app", "db")).toEqual({
    ok: true,
    warning: "no node 'zz'",
  });
  expect(sent).toHaveLength(2);
  expect(useDraft.getState().view?.doc?.edges).toHaveLength(2);
});
