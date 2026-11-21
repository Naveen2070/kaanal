// The load-sweep chart: p99 against load factor, both axes logarithmic, the limit as a labelled dashed
// line, the knee as a labelled line, an error-rate strip beneath and the numbers in a table. A point
// over the limit is a square, a pass is a circle, so the verdict never rests on colour. A separate chunk.

import type { SweepPointDoc, SweepVerdict } from "../../data/api/types";
import {
  byK,
  finding,
  logScale,
  p99Domain,
  sweepGeometry,
  type FirstToSaturate,
} from "./sweepMath";

const M = { l: 48, r: 12, t: 12 }; // plot margins
const W = 420; // plot width
const H = 150; // plot height
const STRIP = 24; // error strip height
const nf = (n: number) => Number(n.toPrecision(3)).toLocaleString("en-US");

export function SweepChart({
  points: input,
  limitMs,
  knee,
  firstToSaturate,
}: {
  points: SweepPointDoc[];
  limitMs: number;
  knee: SweepVerdict | null;
  firstToSaturate: FirstToSaturate;
}) {
  const points = byK(input);
  const g = sweepGeometry(points, limitMs, { w: W, h: H });
  const [lo, hi] = p99Domain(points, limitMs);
  // decade gridlines inside the domain
  const decades: number[] = [];
  if (Number.isFinite(lo) && Number.isFinite(hi) && lo > 0)
    for (let e = Math.ceil(Math.log10(lo)); 10 ** e <= hi; e++) decades.push(10 ** e);
  const yOf = logScale(lo, hi, H, 0);
  const er = (p: SweepPointDoc) => (Number.isFinite(p.error_rate) ? Math.max(0, p.error_rate) : 0);
  const maxErr = Math.max(0.01, ...points.map(er));
  const step = Math.ceil(points.length / 8);
  return (
    <figure className="min-w-0">
      <svg
        viewBox={`0 0 ${M.l + W + M.r} ${M.t + H + 6 + STRIP + 18}`}
        // an svg has no native img tag; the role is the accessible name carrier
        // eslint-disable-next-line jsx-a11y/prefer-tag-over-role
        role="img"
        aria-label={finding(knee, limitMs, firstToSaturate)}
        className="h-auto w-full"
      >
        <g transform={`translate(${M.l} ${M.t})`}>
          {decades.map((d, i) => (
            <g key={`dec${i}`}>
              <line
                x1="0"
                x2={W}
                y1={yOf(d)}
                y2={yOf(d)}
                className="stroke-border"
                strokeWidth="1"
              />
              <text
                x="-6"
                y={yOf(d)}
                textAnchor="end"
                dominantBaseline="middle"
                className="fill-muted text-[10px]"
              >
                {nf(d)}
              </text>
            </g>
          ))}
          <text x="-6" y={-2} textAnchor="end" className="fill-muted text-[10px]">
            p99 ms
          </text>
          {g.dots.length > 0 ? (
            <>
              {limitMs > 0 ? (
                <>
                  <line
                    x1="0"
                    x2={W}
                    y1={g.limitY}
                    y2={g.limitY}
                    className="stroke-negative"
                    strokeDasharray="4 3"
                    strokeWidth="1"
                  />
                  <text
                    x={W}
                    y={g.limitY - 3}
                    textAnchor="end"
                    className="fill-negative text-[10px]"
                  >
                    {`limit p99 ${nf(limitMs)} ms`}
                  </text>
                </>
              ) : null}
              {knee?.kind === "knee" && g.kneeX != null ? (
                <>
                  <line
                    x1={g.kneeX}
                    x2={g.kneeX}
                    y1="0"
                    y2={H}
                    className="stroke-accent"
                    strokeDasharray="2 2"
                    strokeWidth="1"
                  />
                  <text x={g.kneeX + 3} y="9" className="fill-accent text-[10px]">
                    knee
                  </text>
                </>
              ) : null}
              <polyline
                points={g.line}
                fill="none"
                className="stroke-accent"
                strokeWidth="2"
                strokeLinejoin="round"
              />
              {g.dots.map((d, i) =>
                d.over ? (
                  <rect
                    key={`dot${i}`}
                    data-dot
                    x={d.x - 4}
                    y={d.y - 4}
                    width="8"
                    height="8"
                    className="fill-negative"
                  />
                ) : (
                  <circle
                    key={`dot${i}`}
                    data-dot
                    cx={d.x}
                    cy={d.y}
                    r="4"
                    className="fill-positive"
                  />
                ),
              )}
            </>
          ) : null}
          {/* the error-rate strip: bar height is the share, scaled to the largest (at least 1%) */}
          <g transform={`translate(0 ${H + 6})`}>
            <text x="-6" y={STRIP} textAnchor="end" className="fill-muted text-[10px]">
              {`err ${nf(maxErr * 100)}%`}
            </text>
            <line x1="0" x2={W} y1={STRIP} y2={STRIP} className="stroke-border" strokeWidth="1" />
            {g.dots.map((d, i) => {
              const h = (er(points[i] as SweepPointDoc) / maxErr) * STRIP;
              return (
                <rect
                  key={`err${i}`}
                  x={d.x - 3}
                  y={STRIP - h}
                  width="6"
                  height={h}
                  className="fill-negative"
                />
              );
            })}
          </g>
          {g.dots.map((d, i) =>
            i % step === 0 ? (
              <text
                key={`tick${i}`}
                x={d.x}
                y={H + 6 + STRIP + 12}
                textAnchor="middle"
                className="fill-muted text-[10px]"
              >
                {`${nf(d.k)}x`}
              </text>
            ) : null,
          )}
        </g>
      </svg>
      <p className="mt-1 text-xs text-muted">
        circle: within the limits, square: over a limit (p99 or error rate)
      </p>
      <details className="mt-1 text-xs">
        <summary className="cursor-pointer">Points table</summary>
        <table className="mt-1 w-full tabular-nums">
          <caption className="sr-only">Load sweep points</caption>
          <thead>
            <tr>
              {["Load", "req/s", "p50 ms", "p95 ms", "p99 ms", "Errors", "Over limit", "Cause"].map(
                (h) => (
                  <th key={h} scope="col" className="text-left font-medium">
                    {h}
                  </th>
                ),
              )}
            </tr>
          </thead>
          <tbody>
            {points.map((p, i) => (
              <tr key={`row${i}`}>
                <td>{`${nf(p.k)}x`}</td>
                <td>{nf(p.req_s)}</td>
                <td>{nf(p.p50_ms)}</td>
                <td>{nf(p.p95_ms)}</td>
                <td>{nf(p.p99_ms)}</td>
                <td>{`${nf(p.error_rate * 100)}%`}</td>
                <td>{p.over.p99 || p.over.errors ? "yes" : "no"}</td>
                <td>{p.cause ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
