import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { mockFetch } from "../test/fetch";
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
      nodes: { gw: { type: "gateway" }, app: { type: "service" } },
      edges: [{ from: "gw", to: "app" }],
    },
    doc_error: null,
    project_id: "none",
    kind_defaults: { cache: { type: "cache", flavor: "redis" } },
    layout: {},
    layout_note: null,
    layout_dirty: false,
    ...over,
  };
}

async function loaded(first: DraftView) {
  mockFetch([{ json: first }]);
  await useDraft.getState().load();
}

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => vi.unstubAllGlobals());

test("a_patch_is_applied_optimistically_and_rolled_back_on_a_400", async () => {
  await loaded(view());
  mockFetch([{ status: 400, json: { error: "a key is already there" }, delayMs: 20 }]);
  const done = useDraft.getState().patch([{ op: "add_node", name: "c", kind: "cache" }]);
  // shown before the server has answered
  expect(useDraft.getState().view?.doc?.nodes?.c).toEqual({ type: "cache", flavor: "redis" });
  expect(await done).toBe(false);
  expect(useDraft.getState().view?.doc?.nodes?.c).toBeUndefined();
  expect(useDraft.getState().error).toBe("a key is already there");
});

test("the_servers_answer_replaces_the_optimistic_view", async () => {
  await loaded(view());
  const answer = view({ rev: 1, doc: { nodes: { gw: { type: "gateway" } }, edges: [] } });
  mockFetch([{ json: answer }]);
  expect(await useDraft.getState().patch([{ op: "remove", path: ["nodes", "app"] }])).toBe(true);
  expect(useDraft.getState().view).toEqual(answer);
  expect(useDraft.getState().error).toBeNull();
});

test("a_refused_edit_does_not_take_the_later_ones_with_it", async () => {
  await loaded(view());
  const second = view({
    rev: 1,
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service" }, c: { type: "cache" } },
      edges: [{ from: "gw", to: "app" }],
    },
  });
  mockFetch([{ status: 400, json: { error: "no" }, delayMs: 30 }, { json: second }]);
  const a = useDraft.getState().patch([{ op: "remove", path: ["nodes", "gw"] }]);
  const b = useDraft.getState().patch([{ op: "add_node", name: "c", kind: "cache" }]);
  expect(await a).toBe(false);
  expect(await b).toBe(true);
  expect(Object.keys(useDraft.getState().view?.doc?.nodes ?? {})).toEqual(["gw", "app", "c"]);
});

test("a_layout_edit_sends_the_whole_map", async () => {
  await loaded(view({ layout: { gw: [1, 2] } }));
  const sent = mockFetch([{ json: view({ layout: { gw: [1, 2], app: [3, 4] } }) }]);
  await useDraft.getState().setLayout({ app: [3, 4] });
  expect(sent[0]?.body).toEqual({ layout: { gw: [1, 2], app: [3, 4] } });
});

test("a_changed_boot_id_means_the_server_restarted", async () => {
  await loaded(view({ rev: 7 }));
  expect(useDraft.getState().restarted).toBe(false);
  mockFetch([{ json: view({ rev: 0, boot: "b2" }) }]);
  await useDraft.getState().load();
  expect(useDraft.getState().restarted).toBe(true);
});

test("rename_node_moves_edges_and_the_position", async () => {
  await loaded(view({ layout: { app: [5, 6] } }));
  mockFetch([{ status: 400, json: { error: "x" }, delayMs: 20 }]);
  const done = useDraft.getState().patch([{ op: "rename_node", from: "app", to: "api" }]);
  const v = useDraft.getState().view;
  expect(Object.keys(v?.doc?.nodes ?? {})).toEqual(["gw", "api"]);
  expect(v?.doc?.edges?.[0]?.to).toBe("api");
  expect(v?.layout).toEqual({ api: [5, 6] });
  await done;
});

test("append_adds_an_item_to_a_list_at_once", async () => {
  await loaded(view());
  mockFetch([{ status: 400, json: { error: "x" }, delayMs: 20 }]);
  const done = useDraft
    .getState()
    .patch([{ op: "append", path: ["edges"], value: { from: "gw", to: "gw" } }]);
  expect(useDraft.getState().view?.doc?.edges).toHaveLength(2);
  await done;
  expect(useDraft.getState().view?.doc?.edges).toHaveLength(1);
});
