// One continuous timeline of the live run (step 8L): p99, dropped a second and requests a second
// over the chosen window, with a mark at every change you made and at every pause. The charts are
// plain SVG lines on the colour tokens; the marks are listed in words under them, so nothing is
// carried by colour or position alone.

import { useMemo } from "react";
import type { Marker } from "../../data/api/liveTypes";
import { useLive } from "../../data/store/live";
import { timelineSeries, type Point } from "../../data/store/liveMath";
import { fmtMs } from "../../canvas/metrics";

const W = 600;
const H = 56;

const KIND_WORDS: Record<Marker["kind"], string> = {
  param: "You changed",
  chaos: "Chaos",
  structure: "Applied",
  pause: "Paused",
  resume: "Resumed",
  gap: "Missed updates",
};

function Lines({
  title,
  values,
  stroke,
  marks,
  span,
  unit,
}: {
  title: string;
  values: (number | null)[];
  stroke: string;
  marks: number[];
  span: [number, number];
  unit: (n: number) => string;
}) {
  const max = Math.max(1e-9, ...values.filter((v): v is number => v !== null));
  const x = (t: number) => ((t - span[0]) / Math.max(1, span[1] - span[0])) * W;
  const pts = values
    .map((v, i) =>
      v === null
        ? null
        : `${(i / Math.max(1, values.length - 1)) * W},${H - (v / max) * (H - 4) - 2}`,
    )
    .filter((p): p is string => p !== null)
    .join(" ");
  return (
    <figure className="m-0">
      <figcaption className="flex justify-between text-xs text-muted">
        <span>{title}</span>
        <span className="font-mono tabular-nums">peak {unit(max)}</span>
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} className="h-14 w-full">
        <title>{`${title} over the window`}</title>
        {marks.map((t) => (
          <line
            key={t}
            x1={x(t)}
            x2={x(t)}
            y1={0}
            y2={H}
            stroke="var(--muted)"
            strokeDasharray="3 3"
            strokeWidth={1}
          />
        ))}
        <polyline
          fill="none"
          stroke={stroke}
          strokeWidth={1.6}
          strokeLinejoin="round"
          points={pts}
        />
      </svg>
    </figure>
  );
}

const clock = (ms: number) =>
  `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}`;

export function LiveTimeline() {
  const ticks = useLive((s) => s.ticks);
  const markers = useLive((s) => s.markers);
  const series: Point[] = useMemo(() => timelineSeries(ticks), [ticks]);
  if (series.length < 2) {
    return <p className="m-0 text-xs text-muted">The timeline fills in as the run goes on.</p>;
  }
  const span: [number, number] = [series[0]?.t_ms ?? 0, series.at(-1)?.t_ms ?? 1];
  // a run's own first mark (the restart) sits at 0, just before its first tick
  const from = (ticks[0]?.t_ms ?? 0) - 250;
  const inside = markers.filter((m) => m.at_ms >= from && m.at_ms <= span[1]);
  const marks = inside.map((m) => Math.max(m.at_ms, span[0]));
  return (
    <div className="flex flex-col gap-2">
      <Lines
        title="Latency, 99th percentile"
        values={series.map((p) => p.p99Ms)}
        stroke="var(--accent)"
        marks={marks}
        span={span}
        unit={fmtMs}
      />
      <Lines
        title="Dropped a second"
        values={series.map((p) => p.droppedPerS)}
        stroke="var(--negative)"
        marks={marks}
        span={span}
        unit={(n) => n.toFixed(1)}
      />
      <Lines
        title="Requests a second"
        values={series.map((p) => p.reqPerS)}
        stroke="var(--muted)"
        marks={marks}
        span={span}
        unit={(n) => n.toFixed(0)}
      />
      {inside.length > 0 ? (
        <ul
          className="m-0 flex list-none flex-col gap-0.5 p-0 text-xs text-muted"
          aria-label="Changes on the timeline"
        >
          {inside.slice(-6).map((m) => (
            <li key={`${m.at_ms}-${m.kind}-${m.text}`}>
              <span className="font-mono tabular-nums">{clock(m.at_ms)}</span> {KIND_WORDS[m.kind]}
              {m.text && m.kind !== "pause" && m.kind !== "resume" ? `: ${m.text}` : ""}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
