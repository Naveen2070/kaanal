// The dots of one wire: each moves along the wire's own path (a CSS motion path, so the browser
// animates it and a dot costs nothing per frame). They are decoration: hidden from assistive
// technology, and the live strip says the same in words.

import { EdgeLabelRenderer } from "@xyflow/react";
import { useMemo } from "react";
import { useDots } from "./dotStore";

export function DotsOnEdge({ path, edge }: { path: string; edge: number | undefined }) {
  // select the shared array (a stable reference) and filter here: a selector that returns a new
  // array every time would re-render forever
  const all = useDots((s) => s.dots);
  const dots = useMemo(
    () => (edge === undefined ? [] : all.filter((d) => d.edge === edge)),
    [all, edge],
  );
  if (dots.length === 0) return null;
  return (
    <EdgeLabelRenderer>
      {dots.map((d) => (
        <span
          key={d.key}
          aria-hidden
          data-dot
          className="kaanal-dot"
          style={{
            offsetPath: `path("${path}")`,
            animationDuration: `${d.durMs}ms`,
            animationDelay: `${d.delayMs}ms`,
          }}
        />
      ))}
    </EdgeLabelRenderer>
  );
}
