// A node's latency over the run: one coloured cell per (window, latency bucket). The picture is for
// the eye; a table of the buckets below it is what a screen reader gets.

import type { Bucket, Series } from "../api/types";
import { HEAT_ASPECT } from "./Skeletons";
import { bucketLabel, heatmapData, intensity } from "./heatmapData";

const CELL = 8;
const secs = (n: number) => `${Number(n.toPrecision(3))} s`;

export function Heatmap({
  name,
  series,
  buckets,
}: {
  name: string;
  series: Series;
  buckets: Bucket[];
}) {
  const d = heatmapData(series);
  if (d.max === 0) {
    return (
      <p className="text-xs text-muted">
        No requests reached {name} in this run, so there is no latency to draw.
      </p>
    );
  }
  return (
    <figure>
      <svg
        viewBox={`0 0 ${d.cols * CELL} ${d.rows * CELL}`}
        className={`w-full rounded border border-border bg-well ${HEAT_ASPECT}`}
      >
        <title>{`Latency of ${name} over the run: time across, slower hops higher up`}</title>
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
                <title>{`${bucketLabel(buckets, r)}, window ${c + 1}: ${count}`}</title>
              </rect>
            ) : null,
          ),
        )}
      </svg>
      <div aria-hidden className="mt-0.5 flex justify-between text-[11px] tabular-nums text-muted">
        <span>0 s</span>
        <span>{secs((d.cols * series.window_s) / 2)}</span>
        <span>{secs(d.cols * series.window_s)}</span>
      </div>
      <figcaption className="text-xs text-muted">
        Time runs left to right ({d.cols} windows of {Number(series.window_s.toPrecision(3))} s),
        slower hops are higher. Colour is the number of hops, on a log scale.
      </figcaption>
      <table className="sr-only">
        <caption>Hops of {name} by latency</caption>
        <thead>
          <tr>
            <th>Latency</th>
            <th>Hops</th>
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
