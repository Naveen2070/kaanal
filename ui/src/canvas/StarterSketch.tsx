// The picture on a starter's card: an 84 x 56 sketch of its components (a box per component, in the
// colour of its kind) and wires (hairlines; a failing wire dashed in the failure colour). Decorative:
// the card's own words say what the system is, so it is hidden from assistive technology.

import { catColour } from "./nodeKinds";
import { BOX_H, BOX_W, layoutSketch, SKETCH_H, SKETCH_W, type StarterGraph } from "./sketch";

export function StarterSketch({ graph }: { graph: StarterGraph }) {
  const s = layoutSketch(graph);
  return (
    <svg
      aria-hidden
      width={SKETCH_W}
      height={SKETCH_H}
      viewBox={`0 0 ${SKETCH_W} ${SKETCH_H}`}
      className="shrink-0 rounded-lg bg-well"
    >
      {s.wires.map((w, i) => (
        <line
          key={i}
          x1={w.x1}
          y1={w.y1}
          x2={w.x2}
          y2={w.y2}
          strokeWidth={w.failing ? 1.5 : 1}
          strokeDasharray={w.failing ? "3 2" : undefined}
          style={{ stroke: w.failing ? "var(--negative)" : "var(--control)" }}
        />
      ))}
      {s.nodes.map((n) => (
        <rect
          key={n.id}
          x={n.x - BOX_W / 2}
          y={n.y - BOX_H / 2}
          width={BOX_W}
          height={BOX_H}
          rx={2.5}
          style={{ fill: catColour(n.kind) }}
        />
      ))}
    </svg>
  );
}
