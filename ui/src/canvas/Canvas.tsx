// The canvas: the draft's nodes and edges, drawn where the layout says. Everything it changes goes
// through the draft store as a patch (R2.6: the server owns the file); positions are written when
// a drag ends, never during it.

import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Layout } from "../api/types";
import { useDraft } from "../store/draft";
import { useSelected } from "../store/view";
import { usePrefersReducedMotion } from "../first-run/motion";
import { useLayout } from "../layout/useLayout";
import { useRail } from "../layout/rail";
import { LoadEdge } from "./LoadEdge";
import { NodeCard, type CardData } from "./NodeCard";
import { canvasBridge } from "./bridge";
import { nudge, NOT_FOR_NUDGE } from "./nudge";
import { KIND_MIME } from "./Palette";
import { mayCall } from "./rules";
import { removeWires } from "./wires";
import { autoLayout, decodeId, encodeId } from "./edges";
import { notify } from "../toast/toasts";
import { useZoom } from "./zoom";
import { fitPadding, freeSlotIn, isVisible, padsFor, visibleCentre } from "./viewport";

/** Arrow-key moves are saved this long after the last press. */
const NUDGE_SAVE_MS = 300;

/** A fitted view never zooms in past this: a lone card should look like part of a canvas, not fill it. */
const MAX_FIT_ZOOM = 0.9;
/** A card's size in flow units, for deciding whether a new one is in view. */
const CARD_W = 236;
const CARD_H = 190;

const nodeTypes = { card: NodeCard };
const edgeTypes = { load: LoadEdge };

function Flow() {
  const view = useDraft((s) => s.view);
  const addNode = useDraft((s) => s.addNode);
  const connect = useDraft((s) => s.connect);
  const setLayout = useDraft((s) => s.setLayout);
  const [selected, setSelected] = useSelected();
  /** Positions of nodes being dragged right now; not saved until the drag ends. */
  const [live, setLive] = useState<Layout>({});
  /**
   * What React Flow measured of each node. It is handed back with every node: a controlled node
   * without it is treated as unmeasured, and it and its edges are hidden until measured again.
   */
  const [measured, setMeasured] = useState<Record<string, { width?: number; height?: number }>>({});
  const { screenToFlowPosition, fitView, getViewport, setCenter, zoomIn, zoomOut } = useReactFlow();
  const still = usePrefersReducedMotion();
  const mode = useLayout();
  const railOpen = useRail((s) => s.open);
  const pads = padsFor(mode, railOpen);
  const fitOptions = { padding: fitPadding(mode, railOpen), maxZoom: MAX_FIT_ZOOM };
  /** Names given a first layout already, so a refused save cannot loop. */
  const placed = useRef(new Set<string>());

  const doc = view?.doc ?? null;
  const layout = view?.layout;

  // the view is fitted once, on load (the `fitView` prop). A node added later never changes the zoom:
  // the view only moves, and only when the new card would land behind a panel or off screen
  const names = Object.keys(doc?.nodes ?? {});
  const allPlaced = names.every((n) => layout !== undefined && n in layout);
  const known = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!allPlaced) return;
    if (known.current === null) {
      known.current = new Set(names);
      return;
    }
    const fresh = names.filter((n) => !known.current?.has(n));
    known.current = new Set(names);
    const last = fresh[fresh.length - 1];
    const at = last === undefined ? undefined : layout?.[last];
    if (!at) return;
    const rect = document.querySelector(".react-flow")?.getBoundingClientRect();
    const vp = getViewport();
    const left = (rect?.left ?? 0) + vp.x + at[0] * vp.zoom;
    const top = (rect?.top ?? 0) + vp.y + at[1] * vp.zoom;
    const box = { left, top, right: left + CARD_W * vp.zoom, bottom: top + CARD_H * vp.zoom };
    const canvas = rect ?? { left: 0, top: 0, right: 0, bottom: 0 };
    if (!isVisible(box, canvas, pads)) {
      void setCenter(at[0] + CARD_W / 2, at[1] + CARD_H / 2, {
        zoom: vp.zoom,
        duration: still ? 0 : 200,
      });
    }
    // `names` and `pads` are rebuilt every render; the node set and the layout are what matter
    // eslint-disable-next-line react/exhaustive-deps
  }, [JSON.stringify(names), allPlaced, layout]);

  // nodes with no saved position get one, once
  useEffect(() => {
    if (!doc || !layout) return;
    const missing = Object.keys(doc.nodes ?? {}).filter(
      (n) => !(n in layout) && !placed.current.has(n),
    );
    if (missing.length === 0) return;
    missing.forEach((n) => placed.current.add(n));
    const all = autoLayout(doc);
    void setLayout(Object.fromEntries(missing.map((n) => [n, all[n] ?? [0, 0]])));
  }, [doc, layout, setLayout]);

  // folding or opening the rail changes how much canvas is free: refit to it (not on first paint)
  const firstRail = useRef(true);
  useEffect(() => {
    if (firstRail.current) {
      firstRail.current = false;
      return;
    }
    void fitView({ ...fitOptions, duration: still ? 0 : 200 });
    // only the rail's state should trigger this
    // eslint-disable-next-line react/exhaustive-deps
  }, [railOpen]);

  // what the page outside the canvas may ask of it: the visible middle (a clicked palette item lands
  // there, not at the far right) and fit-to-view (the command palette)
  useEffect(() => {
    canvasBridge.centre = () => {
      const r = document.querySelector(".react-flow")?.getBoundingClientRect();
      if (!r) return undefined;
      // the middle of what shows between the panels, not of the whole window
      const c = visibleCentre(r, pads);
      const p = screenToFlowPosition(c);
      // the card's top-left, so its middle is the middle of the visible area
      return [p.x - CARD_W / 2, p.y - CARD_H / 2];
    };
    canvasBridge.region = () => {
      const r = document.querySelector(".react-flow")?.getBoundingClientRect();
      if (!r) return undefined;
      // screen to flow by the viewport itself, so an empty canvas gives the same answer as a full one
      const vp = getViewport();
      const left = (r.left + pads.left - r.left - vp.x) / vp.zoom;
      const top = (r.top + pads.top - r.top - vp.y) / vp.zoom;
      const width = (r.width - pads.left - pads.right) / vp.zoom;
      const height = (r.height - pads.top - pads.bottom) / vp.zoom;
      return { left, top, width, height };
    };
    canvasBridge.fit = () => void fitView({ ...fitOptions, duration: still ? 0 : 200 });
    canvasBridge.zoomIn = () => void zoomIn({ duration: still ? 0 : 150 });
    canvasBridge.zoomOut = () => void zoomOut({ duration: still ? 0 : 150 });
    return () => {
      canvasBridge.centre = () => undefined;
      canvasBridge.region = () => undefined;
      canvasBridge.fit = () => {};
      canvasBridge.zoomIn = () => {};
      canvasBridge.zoomOut = () => {};
    };
    // `pads` and `fitOptions` follow the layout mode, which is what `mode` carries
    // eslint-disable-next-line react/exhaustive-deps
  }, [screenToFlowPosition, fitView, zoomIn, zoomOut, still, mode, railOpen]);

  // arrow keys move the selected node (8 px, Shift 40 px); the node moves on screen at once and the
  // position is saved once the keys pause, not per press. Captured so React Flow's own key move,
  // which never saves, does not also run
  useEffect(() => {
    if (selected === null) return;
    let pos: [number, number] | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => {
      clearTimeout(timer);
      if (pos === null) return;
      const to = pos;
      pos = null;
      setLive({});
      void setLayout({ [selected]: to });
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.(NOT_FOR_NUDGE)) return;
      // only when focus is on the canvas (or nowhere): on a rail tab or a dock button the arrows are theirs
      if (t && t !== document.body && !t.closest?.(".react-flow")) return;
      const from = pos ?? useDraft.getState().view?.layout?.[selected];
      const to = from && nudge(from, e.key, e.shiftKey);
      if (!to) return;
      e.preventDefault();
      e.stopPropagation();
      pos = to;
      setLive((l) => ({ ...l, [selected]: to }));
      clearTimeout(timer);
      timer = setTimeout(flush, NUDGE_SAVE_MS);
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      flush(); // another node was selected, or the canvas left: nothing is lost
    };
  }, [selected, setLayout]);

  /** Wires picked on the canvas (click, or focus and Enter): React Flow needs them to delete one. */
  const [pickedEdges, setPickedEdges] = useState<ReadonlySet<string>>(new Set());

  const { nodes, edges } = useMemo(() => {
    const nodes: Node[] = Object.entries(doc?.nodes ?? {}).map(([name, n]) => ({
      id: encodeId(name),
      type: "card",
      position: (() => {
        const p = live[name] ?? layout?.[name] ?? [0, 0];
        return { x: p[0], y: p[1] };
      })(),
      selected: name === selected,
      // one tab stop per card: its name button (the wrapper would be a second one)
      focusable: false,
      measured: measured[name],
      data: { name, kind: n.type, config: n } satisfies CardData,
    }));
    const edges: Edge[] = (doc?.edges ?? []).map((e, i) => ({
      id: `e${i}`,
      type: "load",
      selected: pickedEdges.has(`e${i}`),
      source: encodeId(String(e.from)),
      target: encodeId(String(e.to)),
      data: { from: String(e.from), to: String(e.to), index: i },
    }));
    return { nodes, edges };
  }, [doc, layout, live, measured, selected, pickedEdges]);

  const onEdgesChange = (changes: EdgeChange[]) => {
    for (const c of changes) {
      if (c.type !== "select") continue;
      setPickedEdges((s) => {
        const next = new Set(s);
        if (c.selected) next.add(c.id);
        else next.delete(c.id);
        return next;
      });
    }
  };

  const onNodesChange = (changes: NodeChange[]) => {
    for (const c of changes) {
      // `add` and `replace` carry a whole node, not an id: the canvas makes neither
      if (!("id" in c)) continue;
      const name = decodeId(c.id);
      if (c.type === "position" && c.position) {
        const at: [number, number] = [c.position.x, c.position.y];
        setLive((l) => ({ ...l, [name]: at }));
      } else if (c.type === "dimensions" && c.dimensions) {
        const dims = c.dimensions;
        setMeasured((m) => ({ ...m, [name]: dims }));
      } else if (c.type === "select" && c.selected) {
        // keyboard selection (focus and Enter) as well as a click
        void setSelected(name);
      }
    }
  };

  /** The drag is over: save where React Flow says the dragged nodes ended, in one request. */
  const onDragStop = (dragged: Node[]) => {
    setLive({});
    if (dragged.length === 0) return;
    void setLayout(
      Object.fromEntries(
        dragged.map((n): [string, [number, number]] => [
          decodeId(n.id),
          [n.position.x, n.position.y],
        ]),
      ),
    );
  };

  /** Delete or Backspace on a selected wire: its entries leave `edges`, highest index first. */
  const onEdgesDelete = (gone: Edge[]) => {
    const at = gone.map((e) => Number(e.id.slice(1))).filter(Number.isInteger);
    if (at.length === 0) return;
    setPickedEdges(new Set()); // wire ids are their place in the list, which shifts
    void removeWires(at);
  };

  const onConnect = async (c: Connection) => {
    notify.clear("connect");
    const r = await connect(decodeId(c.source), decodeId(c.target));
    if (!r.ok) notify.error(r.message, "connect");
    else if (r.warning) notify.warn(r.warning, "connect");
  };

  return (
    <div className="relative h-full">
      <div
        className="relative h-full"
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes(KIND_MIME)) e.preventDefault();
        }}
        onDrop={(e) => {
          const kind = e.dataTransfer.getData(KIND_MIME);
          if (!kind) return;
          e.preventDefault();
          const p = screenToFlowPosition({ x: e.clientX, y: e.clientY });
          void addNode(kind, [p.x, p.y]);
        }}
      >
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          isValidConnection={(c) => {
            const kinds = doc?.nodes ?? {};
            return mayCall(kinds[decodeId(c.source)]?.type, kinds[decodeId(c.target)]?.type);
          }}
          onNodeDragStop={(_, __, dragged) => onDragStop(dragged)}
          onConnect={(c) => void onConnect(c)}
          onNodeClick={(_, n) => void setSelected(decodeId(n.id))}
          onPaneClick={() => void setSelected(null)}
          // only wires can be deleted from the keyboard; a component goes through its panel (with its wires)
          deleteKeyCode={["Backspace", "Delete"]}
          onBeforeDelete={async ({ edges }) => ({ nodes: [], edges })}
          onEdgesDelete={onEdgesDelete}
          fitView
          fitViewOptions={fitOptions}
          minZoom={0.2}
          maxZoom={2}
          // the developer chose to hide the corner credit (2026-11-01); React Flow is MIT, and it is credited in docs/web/studio.md
          proOptions={{ hideAttribution: true }}
          onMove={(_, vp) => useZoom.setState({ zoom: vp.zoom })}
        >
          <Background
            id="minor"
            variant={BackgroundVariant.Lines}
            gap={56}
            lineWidth={0.5}
            color="var(--grid)"
          />
          <Background
            id="major"
            variant={BackgroundVariant.Lines}
            gap={280}
            lineWidth={0.75}
            color="var(--grid)"
            style={{ opacity: 0.9 }}
          />
        </ReactFlow>
        {doc && Object.keys(doc.nodes ?? {}).length === 0 ? (
          // an empty draft: a ghost card pointing at the first step (review C12)
          <div className="pointer-events-none absolute inset-0 grid place-items-center">
            <button
              type="button"
              onClick={() => {
                const r = canvasBridge.region();
                void addNode("gateway", (r && freeSlotIn(r, [])) || [0, 0]);
              }}
              className="pointer-events-auto rounded-2xl border border-dashed border-control px-6 py-4 text-sm text-muted hover:bg-raised"
            >
              Start with a gateway
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function Canvas() {
  return (
    <ReactFlowProvider>
      <Flow />
    </ReactFlowProvider>
  );
}
