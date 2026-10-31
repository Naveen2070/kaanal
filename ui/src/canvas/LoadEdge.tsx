// An edge that shows traffic (spec 3): an orthogonal path with 6px rounded elbows, thicker the more
// requests it carries, the colour of its hotter end, and a dash that flows along it (faster with
// load, still under reduced motion). Colour only repeats what the cards say in words.

import { BaseEdge, getSmoothStepPath, type EdgeProps } from "@xyflow/react";
import { usePrefersReducedMotion } from "../first-run/motion";
import { edgeSpeedS, edgeTone, edgeWidth, type EdgeTone } from "./edges";
import { useFeed } from "./feed";

const STROKE: Record<EdgeTone, string> = {
  muted: "var(--control)",
  warn: "var(--warn-fill)",
  bad: "var(--bad-fill)",
};

export function LoadEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
}: EdgeProps) {
  const feed = useFeed();
  const still = usePrefersReducedMotion();
  const { from, to } = (data ?? {}) as { from?: string; to?: string };
  const a = from === undefined ? undefined : feed.node(from);
  const b = to === undefined ? undefined : feed.node(to);
  const load = feed.peakVisits > 0 && b ? b.visits_per_s / feed.peakVisits : 0;
  const speed = edgeSpeedS(load);
  const [path] = getSmoothStepPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    borderRadius: 6,
  });
  return (
    <BaseEdge
      id={id}
      path={path}
      className={speed !== null && !still ? "kaanal-flow" : undefined}
      style={{
        strokeWidth: edgeWidth(load),
        stroke: STROKE[edgeTone(a?.utilization ?? null, b?.utilization ?? null)],
        ...(speed !== null && !still ? { animationDuration: `${speed}s` } : {}),
      }}
    />
  );
}
