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
import { NodeCard, type CardData } from "./NodeCard";
import { KIND_MIME, Palette } from "./Palette";
import { autoLayout, decodeId, encodeId } from "./edges";

const nodeTypes = { card: NodeCard };

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
  const { screenToFlowPosition } = useReactFlow();
  /** Names given a first layout already, so a refused save cannot loop. */
  const placed = useRef(new Set<string>());

  const doc = view?.doc ?? null;
  const layout = view?.layout;

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
      source: encodeId(String(e.from)),
      target: encodeId(String(e.to)),
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
    <div className="flex h-full">
      <Palette onAdd={(kind) => void addNode(kind)} />
      <div
        className="relative flex-1"
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
          onNodesChange={onNodesChange}
          onNodeDragStop={(_, __, dragged) => onDragStop(dragged)}
          onConnect={(c) => void onConnect(c)}
          onNodeClick={(_, n) => void setSelected(decodeId(n.id))}
          onPaneClick={() => void setSelected(null)}
          deleteKeyCode={null}
          fitView
        >
          <Background />
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
