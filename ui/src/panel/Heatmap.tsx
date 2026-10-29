// A node's latency over the run: one coloured cell per (window, latency bucket). The picture is for
// the eye; a table of the buckets below it is what a screen reader gets.

import type { Bucket, Series } from "../api/types";
import { bucketLabel, heatmapData, intensity } from "./heatmapData";

const CELL = 8;

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
      <p className="text-xs text-neutral-500">
        No requests reached {name} in this run, so there is no latency to draw.
      </p>
    );
  }
  return (
    <figure>
      <svg
        viewBox={`0 0 ${d.cols * CELL} ${d.rows * CELL}`}
        className="w-full rounded border bg-neutral-50"
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
                fill="rgb(37 99 235)"
                fillOpacity={0.1 + 0.9 * intensity(count, d.max)}
              >
                <title>{`${bucketLabel(buckets, r)}, window ${c + 1}: ${count}`}</title>
              </rect>
            ) : null,
          ),
        )}
      </svg>
      <figcaption className="text-xs text-neutral-500">
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
