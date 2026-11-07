// A small picture of a starter system: its components as boxes in the colour of their kind and its wires
// as hairlines, in layers from the front door to the data. A pure function of the starter's own nodes
// and wires, so a test can check every starter has one and the picture cannot drift from the file.
// Layout is the usual layered one for a DAG: a component's column is its longest distance from a
// component nothing calls (Sugiyama's first step), and the components of a column are spread evenly.

export const SKETCH_W = 84;
export const SKETCH_H = 56;
/** The size of one box. */
export const BOX_W = 12;
export const BOX_H = 9;
const PAD_X = 10;
const PAD_Y = 8;

export interface StarterGraph {
  nodes: { id: string; kind: string; copies?: number }[];
  /** Calls, caller first. */
  wires: [string, string][];
  /** Wires drawn dashed in the failure colour (the retry storm's). */
  failing?: [string, string][];
}

export interface SketchNode {
  id: string;
  kind: string;
  x: number;
  y: number;
}

export interface SketchWire {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  failing: boolean;
}

export interface Sketch {
  nodes: SketchNode[];
  wires: SketchWire[];
}

export function layoutSketch(g: StarterGraph): Sketch {
  // each copy of a component is a box of its own, wired the way the component is
  const boxes = g.nodes.flatMap((n) =>
    Array.from({ length: Math.max(1, n.copies ?? 1) }, (_, i) => ({
      key: n.copies && n.copies > 1 ? `${n.id}#${i}` : n.id,
      of: n.id,
      kind: n.kind,
    })),
  );
  const of = (id: string) => boxes.filter((b) => b.of === id);
  const failing = new Set((g.failing ?? []).map(([a, b]) => `${a}>${b}`));
  const links = g.wires.flatMap(([from, to]) =>
    of(from).flatMap((a) =>
      of(to).map((b) => ({ from: a.key, to: b.key, failing: failing.has(`${from}>${to}`) })),
    ),
  );
  // columns: relax the longest distance from a source (the graph is a DAG, so this settles)
  const col = new Map(boxes.map((b) => [b.key, 0]));
  for (let pass = 0; pass < boxes.length; pass++) {
    for (const l of links) {
      col.set(l.to, Math.max(col.get(l.to) ?? 0, (col.get(l.from) ?? 0) + 1));
    }
  }
  const columns = Math.max(...col.values()) + 1;
  const rows = new Map<number, string[]>();
  for (const b of boxes) {
    const c = col.get(b.key) ?? 0;
    rows.set(c, [...(rows.get(c) ?? []), b.key]);
  }
  const at = new Map<string, [number, number]>();
  for (const [c, keys] of rows) {
    keys.forEach((key, r) => {
      const x =
        columns === 1
          ? SKETCH_W / 2
          : PAD_X + BOX_W / 2 + c * ((SKETCH_W - 2 * PAD_X - BOX_W) / (columns - 1));
      const y =
        keys.length === 1
          ? SKETCH_H / 2
          : PAD_Y + BOX_H / 2 + r * ((SKETCH_H - 2 * PAD_Y - BOX_H) / (keys.length - 1));
      at.set(key, [x, y]);
    });
  }
  return {
    nodes: boxes.map((b) => ({
      id: b.key,
      kind: b.kind,
      x: at.get(b.key)![0],
      y: at.get(b.key)![1],
    })),
    wires: links.map((l) => {
      const [x1, y1] = at.get(l.from)!;
      const [x2, y2] = at.get(l.to)!;
      return { x1: x1 + BOX_W / 2, y1, x2: x2 - BOX_W / 2, y2, failing: l.failing };
    }),
  };
}
