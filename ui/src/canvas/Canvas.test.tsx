import { act, cleanup, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { mockFetch } from "../test/fetch";
import { useDraft } from "../store/draft";
import { Canvas } from "./Canvas";
import { KIND_MIME } from "./Palette";

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
}
interface DragNode {
  id: string;
  position: { x: number; y: number };
}
let flow: Props;
vi.mock("@xyflow/react", () => ({
  ReactFlow: (p: Props) => {
    flow = p;
    return <div data-testid="flow" />;
  },
  ReactFlowProvider: ({ children }: { children: unknown }) => <>{children}</>,
  useReactFlow: () => ({ screenToFlowPosition: (p: { x: number; y: number }) => p }),
  Background: () => null,
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
    kind_defaults: { cache: { type: "cache" } },
    layout: { gw: [0, 0], "a#b/c d": [280, 0] },
    layout_note: null,
    layout_dirty: false,
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

test("the_palette_lists_every_kind_and_a_click_adds_one", async () => {
  await show(view());
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

test("a_refused_edit_shows_its_message_on_the_canvas", async () => {
  await show(view());
  mockFetch([{ status: 400, json: { error: "out of range" } }]);
  await act(async () => void useDraft.getState().addNode("cache"));
  expect(screen.getByRole("alert").textContent).toBe("out of range");
});
