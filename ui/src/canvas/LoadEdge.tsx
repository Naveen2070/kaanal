// A wire that shows traffic and its direction (spec 3). Under the lanes sits the wire itself, an
// orthogonal path with rounded elbows: it is what is selected, and it takes a red or amber halo when
// the node it feeds runs hot. On it run one lane per copy of that node, each in the copy's colour,
// each a pulse of dashes that moves toward an arrowhead at the target, faster and heavier as that
// copy takes more load. With reduced motion the lanes stand still. A selected wire shows a button
// to delete it (Delete or Backspace does the same).

import {
  BaseEdge,
  EdgeLabelRenderer,
  getSmoothStepPath,
  useNodes,
  type EdgeProps,
} from "@xyflow/react";
import { X } from "lucide-react";
import { usePrefersReducedMotion } from "../first-run/motion";
import { useDraft } from "../store/draft";
import { carriedNothing, edgeSpeedS, edgeTone } from "./edges";
import { useFeed } from "./feed";
import { laneColour, laneCount, laneLoads, laneOffsets } from "./lanes";
import { replicasOf } from "./metrics";
import { bendX } from "./routing";
import { removeWires } from "./wires";

const HALO = { muted: null, warn: "var(--warn-fill)", bad: "var(--bad-fill)" } as const;

export function LoadEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  selected,
  data,
}: EdgeProps) {
  const feed = useFeed();
  const still = usePrefersReducedMotion();
  const { from, to, index } = (data ?? {}) as { from?: string; to?: string; index?: number };
  const copies = useDraft((s) => (to === undefined ? 1 : replicasOf(s.view?.doc?.nodes?.[to])));
  const a = from === undefined ? undefined : feed.node(from);
  const b = to === undefined ? undefined : feed.node(to);
  const idle = carriedNothing(a, b);
  const n = laneCount(copies);
  const loads = laneLoads(n, b, feed.peakVisits);
  const offsets = laneOffsets(n);
  const halo = HALO[edgeTone(a?.utilization ?? null, b?.utilization ?? null)];

  const geometry = { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition };
  // bend in a gap between the cards, not through one
  const boxes = useNodes().map((nd) => ({
    x: nd.position.x,
    y: nd.position.y,
    w: nd.measured?.width ?? 236,
    h: nd.measured?.height ?? 190,
  }));
  const midX = bendX(sourceX, targetX, sourceY, targetY, boxes);
  const [path, labelX, labelY] = getSmoothStepPath({
    ...geometry,
    borderRadius: 8,
    centerX: midX,
  });
  // each lane bends at its own x beside that, so parallel lanes never run over each other
  const lanes = offsets.map(
    (o) => getSmoothStepPath({ ...geometry, borderRadius: 8, centerX: midX + o })[0],
  );

  return (
    <>
      <defs>
        {Array.from({ length: n }, (_, k) => (
          <marker
            key={k}
            id={`${id}-arrow-${k}`}
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="12"
            markerHeight="12"
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path d="M1 1 L9 5 L1 9 Z" fill={laneColour(k)} />
          </marker>
        ))}
      </defs>
      {/* the wire itself: wide to hit, drawn only when selected */}
      <BaseEdge
        id={id}
        path={path}
        interactionWidth={22}
        style={{
          stroke: selected ? "var(--accent)" : "transparent",
          strokeWidth: selected ? 9 : 1,
          strokeOpacity: selected ? 0.22 : 0,
        }}
      />
      {halo ? (
        <path
          d={path}
          fill="none"
          stroke={halo}
          strokeOpacity={0.35}
          strokeWidth={8 + n * 2}
          pointerEvents="none"
        />
      ) : null}
      {lanes.map((d, k) => {
        const load = loads[k] ?? 0;
        const speed = edgeSpeedS(load);
        const width = 1.8 + 1.6 * load;
        return (
          <g key={k} pointerEvents="none">
            {/* the lane's own colour, faint, so the route reads even with nothing moving */}
            <path
              d={d}
              fill="none"
              stroke={laneColour(k)}
              strokeOpacity={idle ? 0.25 : 0.35}
              strokeWidth={width}
              // a wire that carried nothing in the run is dashed, so it reads as "not used here"
              strokeDasharray={idle ? "3 6" : undefined}
            />
            <path
              d={d}
              fill="none"
              stroke={laneColour(k)}
              strokeWidth={width + 0.6}
              strokeLinecap="round"
              markerEnd={`url(#${id}-arrow-${k})`}
              className={speed !== null && !still ? "kaanal-lane" : undefined}
              strokeOpacity={speed === null ? 0 : 1}
              style={speed !== null && !still ? { animationDuration: `${speed}s` } : undefined}
            />
          </g>
        );
      })}
      {selected && index !== undefined ? (
        <EdgeLabelRenderer>
          <button
            type="button"
            aria-label={`Delete the wire from ${from} to ${to}`}
            title="Delete this wire"
            onClick={() => void removeWires([index])}
            className="nodrag nopan pointer-events-auto absolute flex size-7 items-center justify-center rounded-full border border-negative bg-sheet text-negative shadow-float hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          >
            <X size={15} aria-hidden />
          </button>
        </EdgeLabelRenderer>
      ) : null}
    </>
  );
}
