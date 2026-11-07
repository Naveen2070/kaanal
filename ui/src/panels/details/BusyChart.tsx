// "Busy over the run" for one component: the share of its slots in use in each window, against its
// capacity (the 100% line). A component whose line sits on the 100% line is the one that gives out.
// One 2 px line, straight, with a y scale and a sentence for screen readers; built from the same
// line maths as the charts band. A wire has no busy series (`LinkSeries`), so it shows none.

import { HelpPopover } from "../../overlays/help/HelpPopover";
import { linePoints } from "../charts/line";

const W = 300;
const H = 80;

export function BusyChart({
  name,
  busy,
  windowS,
}: {
  name: string;
  busy: number[];
  windowS: number;
}) {
  const v = busy.filter(Number.isFinite);
  if (v.length === 0) return null;
  const peak = Math.max(...v);
  const top = Math.max(1, peak);
  const full = H - (1 / top) * H;
  const alt = `${name} busy over the run: peak ${Math.round(peak * 100)}% of its slots, last window ${Math.round((v[v.length - 1] ?? 0) * 100)}%`;
  return (
    <figure className="m-0 mb-2 min-w-0">
      <figcaption className="mb-1 flex items-center gap-0.5 text-xs font-medium">
        Busy over the run (100% = every slot in use)
        <HelpPopover id="busy-run" label="Busy over the run" />
      </figcaption>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        className="h-20 w-full overflow-visible"
      >
        <title>{alt}</title>
        <line
          x1="0"
          x2={W}
          y1={full}
          y2={full}
          className="stroke-border"
          strokeWidth="1"
          strokeDasharray="4 3"
          vectorEffect="non-scaling-stroke"
        />
        <polyline
          points={linePoints(v, top, W, H)}
          fill="none"
          className={peak >= 1 ? "stroke-bad-fill" : "stroke-accent"}
          strokeWidth="2"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div aria-hidden className="mt-1 flex justify-between text-[11px] text-muted">
        <span>0 s</span>
        <span>{Number((windowS * v.length).toPrecision(3))} s</span>
      </div>
    </figure>
  );
}
