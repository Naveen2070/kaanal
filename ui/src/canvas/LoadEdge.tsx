// A wire that shows traffic and its direction (spec 3). Under the lanes sits the wire itself, an
// orthogonal path with rounded elbows: it is what is selected, and it takes a red or amber halo when
// the node it feeds runs hot, or a stronger red when the wire itself is full (its bandwidth is the
// limit). On it run one lane per copy of that node, each in the copy's colour,
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
import { usePrefersReducedMotion } from "../overlays/first-run/motion";
import { useDraft } from "../data/store/draft";
import { bandwidthKbOf } from "../panels/details/edgeFields";
import { rateText } from "../panels/details/units";
import { carriedNothing, edgeSpeedS, edgeTone } from "./edges";
import { useFeed } from "./feed";
import { laneColour, laneCount, laneLoads, laneOffsets } from "./lanes";
import { replicasOf } from "./metrics";
import { bendX } from "./routing";
import { DotsOnEdge } from "./dots/DotsOnEdge";
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
  // a wire that is full (its bandwidth is the limit) is red whatever its ends are doing: the nodes
  // behind it sit idle, waiting for what the wire can carry
  const full = from !== undefined && to !== undefined && feed.link?.(from, to)?.saturated === true;
  const halo = full ? HALO.bad : HALO[edgeTone(a?.utilization ?? null, b?.utilization ?? null)];
  // why it is red, on the wire: "280 KB/s · 320%" (its bandwidth, and the demand over it)
  const kb = useDraft((s) => {
    const e = index === undefined ? undefined : s.view?.doc?.edges?.[index];
    return e ? bandwidthKbOf(e) : null;
  });
  const asked =
    full && from !== undefined && to !== undefined
      ? feed.link?.(from, to)?.offered_load
      : undefined;
  const fullLabel =
    full && kb !== null && asked !== undefined && Number.isFinite(asked)
      ? `${rateText(kb)} · ${Math.round(asked * 100).toLocaleString("en-US")}%`
      : null;

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
          data-wire-full={full ? "" : undefined}
          stroke={halo}
          strokeOpacity={full ? 0.6 : 0.35}
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
      <DotsOnEdge path={path} edge={index} />
      {fullLabel ? (
        <EdgeLabelRenderer>
          <span
            data-wire-label
            title="The wire's bandwidth, and the share of it the traffic asks for"
            className="pointer-events-none absolute whitespace-nowrap rounded border border-negative bg-sheet px-1.5 py-0.5 text-[11px] tabular-nums text-negative"
            style={{
              transform: `translate(-50%, ${selected ? "-150%" : "-50%"}) translate(${labelX}px, ${labelY}px)`,
            }}
          >
            {fullLabel}
          </span>
        </EdgeLabelRenderer>
      ) : null}
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
