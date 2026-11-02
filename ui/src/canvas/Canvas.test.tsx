import { act, cleanup, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../data/api/types";
import { mockFetch } from "../test/fetch";
import { useDraft } from "../data/store/draft";
import { Canvas } from "./Canvas";
import { useNotes } from "./notes";
import { KIND_MIME, Palette } from "./Palette";

// React Flow needs a real layout engine; the canvas's own behaviour is what is tested, through the
// props it hands to React Flow.
interface Props {
  nodes: {
    id: string;
    position: { x: number; y: number };
    measured?: { width?: number; height?: number };
    data: { name: string };
  }[];
  edges: { id: string; source: string; target: string }[];
  onNodesChange: (c: unknown[]) => void;
  onNodeDragStop: (e: unknown, n: unknown, dragged: DragNode[]) => void;
  onNodeClick: (e: unknown, n: { id: string }) => void;
  fitView?: boolean;
  edgeTypes?: Record<string, unknown>;
}
interface DragNode {
  id: string;
  position: { x: number; y: number };
}
let flow: Props;
const fit = vi.hoisted(() => vi.fn());
const center = vi.hoisted(() =>
  vi.fn(async (_x: number, _y: number, _o: { zoom: number; duration: number }) => true),
);
vi.mock("@xyflow/react", () => ({
  ReactFlow: (p: Props) => {
    flow = p;
    return <div data-testid="flow" />;
  },
  ReactFlowProvider: ({ children }: { children: unknown }) => <>{children}</>,
  useReactFlow: () => ({
    screenToFlowPosition: (p: { x: number; y: number }) => p,
    fitView: fit,
    getViewport: () => ({ x: 0, y: 0, zoom: 1 }),
    setCenter: center,
  }),
  Background: () => null,
  BackgroundVariant: { Lines: "lines" },
  Controls: () => null,
  Handle: () => null,
  Position: { Left: "left", Right: "right" },
}));
vi.mock("@xyflow/react/dist/style.css", () => ({}));

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b1",
    doc: {
      nodes: { gw: { type: "gateway" }, "a#b/c d": { type: "service" } },
      edges: [{ from: "gw", to: "a#b/c d" }],
    },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: { cache: { type: "cache" } },
    layout: { gw: [0, 0], "a#b/c d": [280, 0] },
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...over,
  };
}

async function show(v: DraftView) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
  render(
    <NuqsAdapter>
      <Canvas />
    </NuqsAdapter>,
  );
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  history.replaceState(null, "", "/__kaanal/studio");
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("names_with_hash_slash_space_and_unicode_are_encoded_on_the_canvas", async () => {
  await show(view());
  const ids = flow.nodes.map((n) => n.id);
  expect(ids).toEqual(["gw", encodeURIComponent("a#b/c d")]);
  expect(flow.nodes[1]?.data.name).toBe("a#b/c d");
  expect(flow.edges[0]).toMatchObject({ source: "gw", target: encodeURIComponent("a#b/c d") });
  // selecting one puts its real name in the address bar
  await act(async () => flow.onNodeClick({}, { id: ids[1] as string }));
  await vi.waitFor(() => expect(new URLSearchParams(location.search).get("node")).toBe("a#b/c d"));
});

test("positions_are_saved_to_the_layout_after_a_drag_ends_not_during", async () => {
  await show(view());
  const sent = mockFetch([{ json: view({ layout: { gw: [5, 6], "a#b/c d": [280, 0] } }) }]);
  for (const x of [1, 2, 3, 4, 5]) {
    await act(async () =>
      flow.onNodesChange([{ type: "position", id: "gw", position: { x, y: 6 } }]),
    );
  }
  expect(sent).toHaveLength(0);
  expect(flow.nodes[0]?.position).toEqual({ x: 5, y: 6 });
  // the end of the drag carries the last position even if it was never rendered
  await act(async () => flow.onNodeDragStop({}, {}, [{ id: "gw", position: { x: 5, y: 6 } }]));
  expect(sent).toHaveLength(1);
  expect(sent[0]?.body).toEqual({ layout: { gw: [5, 6], "a#b/c d": [280, 0] } });
});

test("nodes_without_a_layout_get_a_left_to_right_layout_once", async () => {
  const bare = view({ layout: {} });
  mockFetch([{ json: bare }]);
  await useDraft.getState().load();
  const sent = mockFetch([{ status: 400, json: { error: "no" } }]);
  await act(async () => {
    render(
      <NuqsAdapter>
        <Canvas />
      </NuqsAdapter>,
    );
  });
  // refused, and still only asked once: a failed save must not loop
  await act(async () => useDraft.setState({ view: { ...bare } }));
  expect(sent).toHaveLength(1);
  const body = sent[0]?.body as { layout: Record<string, [number, number]> };
  expect(Object.keys(body.layout).sort()).toEqual(["a#b/c d", "gw"]);
  expect(body.layout.gw?.[0]).toBeLessThan(body.layout["a#b/c d"]?.[0] ?? 0);
});

test("a_palette_drop_adds_a_node_where_it_landed", async () => {
  await show(view());
  const sent = mockFetch([{ json: view() }]);
  const target = screen.getByTestId("flow").parentElement as HTMLElement;
  // jsdom's drag events carry no coordinates: set them on the event
  const drop = createEvent.drop(target, {
    dataTransfer: { getData: (t: string) => (t === KIND_MIME ? "cache" : ""), types: [KIND_MIME] },
  });
  Object.defineProperties(drop, { clientX: { value: 120 }, clientY: { value: 80 } });
  await act(async () => fireEvent(target, drop));
  expect(sent[0]?.body).toMatchObject({
    patch: [{ op: "add_node", name: "cache", kind: "cache" }],
    layout: { cache: [120, 80] },
  });
});

// the palette is a Shell slot now (App wires its onAdd to the store); this is that wiring
test("the_palette_lists_every_kind_and_a_click_adds_one", async () => {
  await show(view());
  render(<Palette onAdd={(kind) => void useDraft.getState().addNode(kind)} />);
  const sent = mockFetch([{ json: view() }]);
  expect(screen.getAllByRole("button")).toHaveLength(7);
  await act(async () => fireEvent.click(screen.getByRole("button", { name: /cache/i })));
  expect(sent[0]?.body).toMatchObject({
    patch: [{ op: "add_node", name: "cache", kind: "cache" }],
  });
});

test("measured_sizes_are_handed_back_so_nodes_stay_visible_after_every_change", async () => {
  await show(view());
  await act(async () =>
    flow.onNodesChange([{ type: "dimensions", id: "gw", dimensions: { width: 160, height: 60 } }]),
  );
  expect(flow.nodes[0]?.measured).toEqual({ width: 160, height: 60 });
  // a drag move rebuilds the nodes: the size must survive it
  await act(async () =>
    flow.onNodesChange([{ type: "position", id: "gw", position: { x: 9, y: 9 } }]),
  );
  expect(flow.nodes[0]?.measured).toEqual({ width: 160, height: 60 });
  expect(flow.nodes[0]?.position).toEqual({ x: 9, y: 9 });
});

test("selecting_with_the_keyboard_selects_the_node", async () => {
  await show(view());
  await act(async () => flow.onNodesChange([{ type: "select", id: "gw", selected: true }]));
  await vi.waitFor(() => expect(new URLSearchParams(location.search).get("node")).toBe("gw"));
});

test("the_canvas_draws_no_notice_of_its_own", async () => {
  await show(view());
  mockFetch([{ status: 400, json: { error: "out of range" } }]);
  await act(async () => void useDraft.getState().addNode("cache"));
  // a refused edit is reported by the notifications (toast/bind.ts), never as a box on the canvas
  expect(screen.queryByRole("alert")).toBeNull();
});

// ---- T2.3: fit view ----

test("the_view_fits_on_load_and_never_zooms_again_when_a_node_is_added", async () => {
  fit.mockClear();
  center.mockClear();
  await show(view());
  expect(flow.fitView).toBe(true); // on load, by React Flow itself
  expect((flow as unknown as { fitViewOptions: { maxZoom: number } }).fitViewOptions.maxZoom).toBe(
    0.9,
  );
  expect(fit).not.toHaveBeenCalled();

  const doc = view().doc!;
  const withCache = { ...doc, nodes: { ...doc.nodes, cache: { type: "cache" } } };
  // a node arrives with its position (jsdom has no layout, so it counts as out of view): the view
  // moves to it and keeps its zoom; it is never fitted again
  await act(async () =>
    useDraft.setState({
      view: view({ doc: withCache, layout: { ...view().layout, cache: [560, 0] } }),
    }),
  );
  expect(fit).not.toHaveBeenCalled();
  expect(center).toHaveBeenCalledTimes(1);
  expect(center.mock.calls[0]?.[2]).toMatchObject({ zoom: 1 });
  // an unrelated change moves nothing
  act(() => useDraft.setState({ view: view({ rev: 5 }) }));
  expect(center).toHaveBeenCalledTimes(1);
});

test("edges_are_drawn_by_the_load_edge_with_their_ends_named", async () => {
  await show(view());
  expect(flow.edgeTypes).toHaveProperty("load");
  const e = (flow as unknown as { edges: { type: string; data: object }[] }).edges[0]!;
  expect(e.type).toBe("load");
  expect(e.data).toEqual({ from: "gw", to: "a#b/c d", index: 0 });
});

test("arrow_keys_move_a_selected_node_on_screen_at_once_and_save_once_after_a_pause", async () => {
  vi.useFakeTimers();
  try {
    history.replaceState(null, "", "/__kaanal/studio?node=gw");
    mockFetch([{ json: view() }]);
    await useDraft.getState().load();
    render(
      <NuqsAdapter>
        <Canvas />
      </NuqsAdapter>,
    );
    const sent = mockFetch([{ json: view({ layout: { gw: [24, 0], "a#b/c d": [280, 0] } }) }]);
    for (const k of ["ArrowRight", "ArrowRight", "ArrowRight"]) {
      act(() => void fireEvent.keyDown(document.body, { key: k }));
    }
    // moved on screen already, nothing sent yet
    expect(flow.nodes.find((n) => n.data.name === "gw")?.position).toEqual({ x: 24, y: 0 });
    expect(sent).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTimeAsync(400));
    await act(async () => {
      await Promise.resolve();
    });
    expect(sent).toHaveLength(1);
    expect(sent[0]?.body).toEqual({ layout: { gw: [24, 0], "a#b/c d": [280, 0] } });
  } finally {
    vi.useRealTimers();
  }
});

test("arrow_keys_in_a_slider_or_field_do_not_move_the_node", async () => {
  history.replaceState(null, "", "/__kaanal/studio?node=gw");
  await show(view());
  const sent = mockFetch([{ json: view() }]);
  const slider = document.createElement("span");
  slider.setAttribute("role", "slider");
  document.body.append(slider);
  fireEvent.keyDown(slider, { key: "ArrowRight" });
  slider.remove();
  expect(sent).toHaveLength(0);
  expect(flow.nodes.find((n) => n.data.name === "gw")?.position).toEqual({ x: 0, y: 0 });
});

interface Deletable {
  deleteKeyCode: string[] | null;
  onBeforeDelete: (d: {
    nodes: unknown[];
    edges: unknown[];
  }) => Promise<{ nodes: unknown[]; edges: unknown[] }>;
  onEdgesDelete: (e: { id: string }[]) => void;
}

test("delete_and_backspace_remove_a_selected_wire_and_never_a_component", async () => {
  await show(view());
  const f = flow as unknown as Deletable;
  expect(f.deleteKeyCode).toEqual(["Backspace", "Delete"]);
  const kept = await f.onBeforeDelete({ nodes: [{ id: "n" }], edges: [{ id: "e0" }] });
  expect(kept).toEqual({ nodes: [], edges: [{ id: "e0" }] });
});

test("deleted_wires_leave_the_edge_list_highest_index_first_in_one_patch", async () => {
  await show(view());
  const sent = mockFetch([{ json: view() }]);
  (flow as unknown as Deletable).onEdgesDelete([{ id: "e0" }, { id: "e2" }, { id: "e1" }]);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      { op: "remove", path: ["edges", 2] },
      { op: "remove", path: ["edges", 1] },
      { op: "remove", path: ["edges", 0] },
    ],
  });
});

test("arrow_keys_on_a_button_outside_the_canvas_do_not_move_the_node", async () => {
  history.replaceState(null, "", "/__kaanal/studio?node=gw");
  await show(view());
  const sent = mockFetch([{ json: view() }]);
  const tab = document.createElement("button"); // a rail tab: the arrows are the tab list's
  document.body.append(tab);
  fireEvent.keyDown(tab, { key: "ArrowRight" });
  tab.remove();
  expect(sent).toHaveLength(0);
  expect(flow.nodes.find((n) => n.data.name === "gw")?.position).toEqual({ x: 0, y: 0 });
});

test("a_wire_picked_on_the_canvas_stays_selected_so_delete_can_find_it", async () => {
  await show(view());
  const f = flow as unknown as {
    onEdgesChange: (c: { type: string; id: string; selected: boolean }[]) => void;
    edges: { id: string; selected?: boolean }[];
  };
  expect(f.edges.some((e) => e.selected)).toBe(false);
  act(() => f.onEdgesChange([{ type: "select", id: "e0", selected: true }]));
  expect((flow as unknown as typeof f).edges.find((e) => e.id === "e0")?.selected).toBe(true);
  act(() => f.onEdgesChange([{ type: "select", id: "e0", selected: false }]));
  expect((flow as unknown as typeof f).edges.find((e) => e.id === "e0")?.selected).toBe(false);
});

test("captions_are_drawn_beside_the_cards_move_with_a_drag_and_are_never_the_selected_component", async () => {
  localStorage.clear();
  useNotes.setState({ project: null, notes: [] });
  localStorage.setItem(
    "kaanal-notes-proj",
    JSON.stringify([{ id: "note:1", text: "the only entry", at: [50, 60] }]),
  );
  await show(view({ project_id: "proj" }));
  const note = flow.nodes.find((n) => n.id === "note:1") as unknown as {
    type: string;
    position: { x: number; y: number };
    selectable: boolean;
    data: { text: string };
  };
  expect(note).toMatchObject({
    type: "note",
    position: { x: 50, y: 60 },
    selectable: false,
    data: { text: "the only entry" },
  });
  // it moves on screen at once, and the new place is kept when the drag ends
  await act(async () =>
    flow.onNodesChange([{ type: "position", id: "note:1", position: { x: 70, y: 80 } }]),
  );
  expect(flow.nodes.find((n) => n.id === "note:1")?.position).toEqual({ x: 70, y: 80 });
  const sent = mockFetch([{ json: view() }]);
  await act(async () =>
    flow.onNodeDragStop({}, {}, [{ id: "note:1", position: { x: 70, y: 80 } }]),
  );
  expect(useNotes.getState().notes[0]?.at).toEqual([70, 80]);
  expect(sent).toHaveLength(0); // a caption is not part of the layout file
  // clicking it does not select a component named "note:1"
  await act(async () => flow.onNodeClick({}, { id: "note:1" }));
  expect(new URLSearchParams(location.search).get("node")).toBeNull();
});
