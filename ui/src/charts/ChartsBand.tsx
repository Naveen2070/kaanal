// The charts band under the canvas (spec 6): p99 latency, goodput and failures over the run, one hue
// each, 120 px plots with a y scale and gridlines, straight 2 px lines, no legend (one series per
// chart), and a sentence for each so a screen reader gets the numbers. Collapsible; the choice is
// remembered. A separate chunk.

import { useState } from "react";
import { PanelBottomClose, PanelBottomOpen } from "lucide-react";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { systemCounts } from "./counts";
import { linePoints, niceTicks, smooth } from "./line";

const KEY = "kaanal.studio.charts.open";
const open0 = () => {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
};

const nf = (n: number) => Number(n.toPrecision(3)).toLocaleString("en-US");

function Chart({
  title,
  unit,
  values,
  stroke,
  windowS,
  trend = false,
}: {
  title: string;
  unit: string;
  values: number[];
  stroke: string;
  /** Seconds one point stands for, for the time axis. */
  windowS?: number;
  /** Draw a moving average (p99 steps between histogram buckets). */
  trend?: boolean;
}) {
  const drawn = trend ? smooth(values) : values;
  const max = Math.max(0, ...drawn.filter(Number.isFinite));
  const ticks = niceTicks(max, 3);
  const top = ticks[ticks.length - 1] as number;
  const last = values.length > 0 ? (values[values.length - 1] as number) : 0;
  const rawMax = Math.max(0, ...values.filter(Number.isFinite));
  const alt = `${title}: peak ${nf(rawMax)} ${unit}, last window ${nf(last)} ${unit}, over ${values.length} windows`;
  return (
    <figure className="min-w-0 flex-1">
      <figcaption className="mb-1 text-xs font-medium">
        {title}
        {trend ? <span className="font-normal text-muted"> (ms, smoothed)</span> : null}
      </figcaption>
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
            points={linePoints(drawn, top, 300, 120)}
            fill="none"
            className={stroke}
            strokeWidth="2"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      </div>
      {windowS ? (
        <div aria-hidden className="ml-10 mt-1 flex justify-between text-[11px] text-muted">
          <span>0 s</span>
          <span>{nf(Number((windowS * values.length).toPrecision(3)))} s</span>
        </div>
      ) : null}
    </figure>
  );
}

/** The system at a glance: how many parts, of what sort, how many wires, and the run's simulated time. */
function StatsPill() {
  const doc = useDraft((s) => s.view?.doc);
  const windowS = useRun((s) => s.latest?.summary?.window_s);
  const c = systemCounts(doc);
  const parts = [
    c.traffic > 0 && `${c.traffic} traffic`,
    c.compute > 0 && `${c.compute} compute`,
    c.data > 0 && `${c.data} data`,
    c.external > 0 && `${c.external} external`,
  ].filter(Boolean);
  // one short plain line, sentence case, never wrapped; the sorts of parts are in the tooltip
  const line = [
    `${c.components} ${c.components === 1 ? "component" : "components"}`,
    `${c.connections} ${c.connections === 1 ? "connection" : "connections"}`,
    windowS ? `${Number(windowS.toPrecision(3))} s simulated` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const sorts = parts.length > 0 ? ` Of the components: ${parts.join(", ")}.` : "";
  return (
    <p
      data-testid="system-stats"
      title={`What the draft holds.${sorts} The time is how much simulated time the last run covered.`}
      className="m-0 text-xs whitespace-nowrap text-muted tabular-nums"
    >
      {line}
    </p>
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
    <section
      aria-label="Charts"
      className={`pointer-events-auto shrink-0 rounded-2xl border border-border bg-sheet shadow-float ${open ? "w-full" : "w-fit"}`}
    >
      <div className="flex items-center gap-3 pr-4">
        <button
          type="button"
          aria-expanded={open}
          onClick={toggle}
          className="flex h-9 shrink-0 items-center gap-2 rounded-2xl px-4 text-xs font-medium focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
        >
          {open ? (
            <PanelBottomClose size={15} aria-hidden />
          ) : (
            <PanelBottomOpen size={15} aria-hidden />
          )}
          {open ? "Hide charts" : "Show charts"}
        </button>
        <span aria-hidden className="h-4 w-px bg-border" />
        <StatsPill />
      </div>
      {open ? (
        series ? (
          <div className="flex gap-4 px-3 pb-3">
            <Chart
              title="p99 latency"
              unit="ms"
              values={series.p99_ms}
              stroke="stroke-accent"
              windowS={series.window_s}
              trend
            />
            <Chart
              title={`Goodput, done per ${win} window`}
              unit="requests"
              values={series.done}
              stroke="stroke-positive"
              windowS={series.window_s}
            />
            <Chart
              title={`Failures, failed per ${win} window`}
              unit="requests"
              values={series.failed}
              stroke="stroke-negative"
              windowS={series.window_s}
            />
          </div>
        ) : (
          <p className="px-3 pb-3 text-xs text-muted">The charts appear after the first run.</p>
        )
      ) : null}
    </section>
  );
}
