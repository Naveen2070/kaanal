// A node's latency over the run, or a wire's time on the wire (transfer and time lost sharing it): one coloured cell
// per (window, latency bucket). The picture is for the eye; a table of the buckets below it is what a
// screen reader gets.

import type { Bucket, Series } from "../../data/api/types";
import { HEAT_ASPECT } from "./Skeletons";
import { bucketLabel, heatmapData, intensity } from "./heatmapData";

const CELL = 8;
// the scale reads "128 ms", not "<= 128 ms" (the tooltip and the table keep the full words)
const axis = (label: string) => label.replace("<= ", "");
const secs = (n: number) => `${Number(n.toPrecision(3))} s`;

export function Heatmap({
  name,
  series,
  buckets,
  wire = false,
}: {
  name: string;
  series: Pick<Series, "window_s" | "latency">;
  buckets: Bucket[];
  /** A wire, not a component: its bodies, not hops. */
  wire?: boolean;
}) {
  const thing = wire ? "bodies" : "hops";
  const d = heatmapData(series);
  if (d.max === 0) {
    return (
      <p className="text-xs text-muted">
        {wire
          ? `Nothing crossed ${name} in this run, so there is no time on the wire to draw.`
          : `No requests reached ${name} in this run, so there is no latency to draw.`}
      </p>
    );
  }
  return (
    <figure>
      <div className="flex items-stretch gap-1">
        {/* the scale: slowest bucket at the top, the middle one, and the fastest at the bottom */}
        <div
          aria-hidden
          className="flex w-14 shrink-0 flex-col justify-between text-right text-[11px] tabular-nums text-muted"
        >
          <span className="whitespace-nowrap">{axis(bucketLabel(buckets, d.rows - 1))}</span>
          <span className="whitespace-nowrap">
            {axis(bucketLabel(buckets, Math.floor((d.rows - 1) / 2)))}
          </span>
          <span className="whitespace-nowrap">{axis(bucketLabel(buckets, 0))}</span>
        </div>
        <svg
          viewBox={`0 0 ${d.cols * CELL} ${d.rows * CELL}`}
          className={`min-w-0 flex-1 rounded border border-border bg-well ${HEAT_ASPECT}`}
        >
          <title>{`${wire ? "Time on the wire" : "Latency"} of ${name} over the run: time across, slower ${thing} higher up`}</title>
          {d.cells.map((row, r) =>
            row.map((count, c) =>
              count > 0 ? (
                <rect
                  key={`${r}-${c}`}
                  x={c * CELL}
                  // row 0 (fastest) at the bottom
                  y={(d.rows - 1 - r) * CELL}
                  width={CELL}
                  height={CELL}
                  className="fill-accent"
                  fillOpacity={0.1 + 0.9 * intensity(count, d.max)}
                >
                  <title>{`${bucketLabel(buckets, r)}, window ${c + 1}: ${count} ${thing}`}</title>
                </rect>
              ) : null,
            ),
          )}
        </svg>
      </div>
      <div
        aria-hidden
        className="mt-0.5 ml-[60px] flex justify-between text-[11px] tabular-nums text-muted"
      >
        <span>0 s</span>
        <span>{secs((d.cols * series.window_s) / 2)}</span>
        <span>{secs(d.cols * series.window_s)}</span>
      </div>
      <figcaption className="text-xs text-muted">
        Time runs left to right ({d.cols} windows of {Number(series.window_s.toPrecision(3))} s),
        slower {thing} are higher. Colour is the number of {thing}, on a log scale.
        {wire
          ? " Time on the wire is the transfer plus the time lost while other bodies shared the wire."
          : ""}
      </figcaption>
      <table className="sr-only">
        <caption>
          {wire ? `Bodies on ${name} by time on the wire` : `Hops of ${name} by latency`}
        </caption>
        <thead>
          <tr>
            <th>Latency</th>
            <th>{wire ? "Bodies" : "Hops"}</th>
          </tr>
        </thead>
        <tbody>
          {d.cells.map((row, r) => (
            <tr key={r}>
              <td>{bucketLabel(buckets, r)}</td>
              <td>{row.reduce((a, b) => a + b, 0)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}
