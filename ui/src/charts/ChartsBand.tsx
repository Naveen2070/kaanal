// The charts band under the canvas (spec 6): p99 latency, goodput and failures over the run, one hue
// each, 120 px plots with a y scale and gridlines, straight 2 px lines, no legend (one series per
// chart), and a sentence for each so a screen reader gets the numbers. Collapsible; the choice is
// remembered. A separate chunk.

import { useState } from "react";
import { useRun } from "../store/run";
import { linePoints, niceTicks } from "./line";

const KEY = "kaanal.studio.charts.open";
const open0 = () => {
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
};

const nf = (n: number) => Number(n.toPrecision(3)).toLocaleString("en-US");

function Chart({
  title,
  unit,
  values,
  stroke,
}: {
  title: string;
  unit: string;
  values: number[];
  stroke: string;
}) {
  const max = Math.max(0, ...values.filter(Number.isFinite));
  const ticks = niceTicks(max, 3);
  const top = ticks[ticks.length - 1] as number;
  const last = values.length > 0 ? (values[values.length - 1] as number) : 0;
  const alt = `${title}: peak ${nf(max)} ${unit}, last window ${nf(last)} ${unit}, over ${values.length} windows`;
  return (
    <figure className="min-w-0 flex-1">
      <figcaption className="mb-1 text-xs font-medium">{title}</figcaption>
      <div className="flex gap-1">
        <div aria-hidden className="relative h-[120px] w-9 text-[11px] tabular-nums text-muted">
          {ticks.map((t) => (
            <span
              key={t}
              className="absolute right-0 -translate-y-1/2"
              style={{ top: `${(1 - t / top) * 100}%` }}
            >
              {nf(t)}
            </span>
          ))}
        </div>
        <svg
          viewBox="0 0 300 120"
          preserveAspectRatio="none"
          className="h-[120px] min-w-0 flex-1 overflow-visible"
        >
          <title>{alt}</title>
          {ticks.map((t) => (
            <line
              key={t}
              x1="0"
              x2="300"
              y1={120 - (t / top) * 120}
              y2={120 - (t / top) * 120}
              className="stroke-border"
              strokeWidth="1"
              vectorEffect="non-scaling-stroke"
            />
          ))}
          <polyline
            points={linePoints(values, top, 300, 120)}
            fill="none"
            className={stroke}
            strokeWidth="2"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      </div>
    </figure>
  );
}

export function ChartsBand() {
  const series = useRun((s) => s.latest?.summary?.series);
  const [open, setOpen] = useState(open0);
  const toggle = () => {
    setOpen(!open);
    try {
      localStorage.setItem(KEY, open ? "0" : "1");
    } catch {
      // nowhere to remember it: it opens again next time
    }
  };
  const win = series ? `${Number(series.window_s.toPrecision(3))} s` : "";
  return (
    <section aria-label="Charts" className="shrink-0 border-t border-border">
      <button
        type="button"
        aria-expanded={open}
        onClick={toggle}
        className="flex h-9 w-full items-center px-3 text-xs font-medium focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
      >
        {open ? "Hide charts" : "Show charts"}
      </button>
      {open ? (
        series ? (
          <div className="flex gap-4 px-3 pb-3">
            <Chart title="p99 latency" unit="ms" values={series.p99_ms} stroke="stroke-accent" />
            <Chart
              title={`Goodput, done per ${win} window`}
              unit="requests"
              values={series.done}
              stroke="stroke-positive"
            />
            <Chart
              title={`Failures, failed per ${win} window`}
              unit="requests"
              values={series.failed}
              stroke="stroke-negative"
            />
          </div>
        ) : (
          <p className="px-3 pb-3 text-xs text-muted">The charts appear after the first run.</p>
        )
      ) : null}
    </section>
  );
}
