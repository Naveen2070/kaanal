// The canvas: the draft's nodes and edges, drawn where the layout says. Everything it changes goes
// through the draft store as a patch (R2.6: the server owns the file); positions are written when
// a drag ends, never during it.

import {
  Background,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Layout } from "../api/types";
import { useDraft } from "../store/draft";
import { useSelected } from "../store/view";
import { usePrefersReducedMotion } from "../first-run/motion";
import { LoadEdge } from "./LoadEdge";
import { NodeCard, type CardData } from "./NodeCard";
import { canvasBridge } from "./bridge";
import { nudge, NOT_FOR_NUDGE } from "./nudge";
import { KIND_MIME } from "./Palette";
import { autoLayout, decodeId, encodeId } from "./edges";

const nodeTypes = { card: NodeCard };
const edgeTypes = { load: LoadEdge };

function Flow() {
  const view = useDraft((s) => s.view);
  const addNode = useDraft((s) => s.addNode);
  const connect = useDraft((s) => s.connect);
  const setLayout = useDraft((s) => s.setLayout);
  const [selected, setSelected] = useSelected();
  const [notice, setNotice] = useState<string | null>(null);
  /** Positions of nodes being dragged right now; not saved until the drag ends. */
  const [live, setLive] = useState<Layout>({});
  /**
   * What React Flow measured of each node. It is handed back with every node: a controlled node
   * without it is treated as unmeasured, and it and its edges are hidden until measured again.
   */
  const [measured, setMeasured] = useState<Record<string, { width?: number; height?: number }>>({});
  const storeError = useDraft((s) => s.error);
  const { screenToFlowPosition, fitView } = useReactFlow();
  const still = usePrefersReducedMotion();
  /** Names given a first layout already, so a refused save cannot loop. */
  const placed = useRef(new Set<string>());

  const doc = view?.doc ?? null;
  const layout = view?.layout;

  // fit view on load (the `fitView` prop) and after a node is added, once it has its position, so
  // no card is left cut off at an edge
  const count = Object.keys(doc?.nodes ?? {}).length;
  const allPlaced = Object.keys(doc?.nodes ?? {}).every((n) => layout !== undefined && n in layout);
  const lastCount = useRef(count);
  const wantFit = useRef(false);
  useEffect(() => {
    if (count > lastCount.current) wantFit.current = true;
    lastCount.current = count;
    if (wantFit.current && allPlaced) {
      wantFit.current = false;
      void fitView({ padding: 0.2, duration: still ? 0 : 200 });
    }
  }, [count, allPlaced, fitView, still]);

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

  // what the page outside the canvas may ask of it: the visible middle (a clicked palette item lands
  // there, not at the far right) and fit-to-view (the command palette)
  useEffect(() => {
    canvasBridge.centre = () => {
      const r = document.querySelector(".react-flow")?.getBoundingClientRect();
      if (!r) return undefined;
      const p = screenToFlowPosition({ x: r.left + r.width / 2, y: r.top + r.height / 2 });
      // the card's top-left, so its middle is the middle of the view
      return [p.x - 118, p.y - 55];
    };
    canvasBridge.fit = () => void fitView({ padding: 0.2, duration: still ? 0 : 200 });
    return () => {
      canvasBridge.centre = () => undefined;
      canvasBridge.fit = () => {};
    };
  }, [screenToFlowPosition, fitView, still]);

  // arrow keys move the selected node (8 px, Shift 40 px) and save where it ends; captured so React
  // Flow's own key move, which never saves, does not also run
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (selected === null || e.ctrlKey || e.metaKey || e.altKey) return;
      if ((e.target as HTMLElement | null)?.closest?.(NOT_FOR_NUDGE)) return;
      const at = layout?.[selected];
      const to = at && nudge(at, e.key, e.shiftKey);
      if (!to) return;
      e.preventDefault();
      e.stopPropagation();
      void setLayout({ [selected]: to });
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [selected, layout, setLayout]);

  const { nodes, edges } = useMemo(() => {
    const nodes: Node[] = Object.entries(doc?.nodes ?? {}).map(([name, n]) => ({
      id: encodeId(name),
      type: "card",
      position: (() => {
        const p = live[name] ?? layout?.[name] ?? [0, 0];
        return { x: p[0], y: p[1] };
      })(),
      selected: name === selected,
      measured: measured[name],
      data: { name, kind: n.type, config: n } satisfies CardData,
    }));
    const edges: Edge[] = (doc?.edges ?? []).map((e, i) => ({
      id: `e${i}`,
      type: "load",
      source: encodeId(String(e.from)),
      target: encodeId(String(e.to)),
      data: { from: String(e.from), to: String(e.to) },
    }));
    return { nodes, edges };
  }, [doc, layout, live, measured, selected]);

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

  const onConnect = async (c: Connection) => {
    setNotice(null);
    const r = await connect(decodeId(c.source), decodeId(c.target));
    if (!r.ok) setNotice(r.message);
    else if (r.warning) setNotice(r.warning);
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
        {(notice ?? storeError) ? (
          <div
            role="alert"
            className="absolute left-2 top-2 z-10 max-w-md rounded-md bg-red-50 px-3 py-2 text-sm text-red-800 shadow"
          >
            {notice ?? storeError}
          </div>
        ) : null}
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodesChange={onNodesChange}
          onNodeDragStop={(_, __, dragged) => onDragStop(dragged)}
          onConnect={(c) => void onConnect(c)}
          onNodeClick={(_, n) => void setSelected(decodeId(n.id))}
          onPaneClick={() => void setSelected(null)}
          deleteKeyCode={null}
          fitView
        >
          <Background color="var(--border)" />
          <Controls showInteractive={false} />
        </ReactFlow>
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
