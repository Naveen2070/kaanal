// Scale and path maths for the charts band. Pure, so it is tested without drawing.

/** Round tick values from 0 up to a top tick at or above `max`: (87, 4) gives 0, 25, 50, 75, 100. */
export function niceTicks(max: number, count: number): number[] {
  if (!Number.isFinite(max) || max <= 0 || count < 1) return [0, 1];
  const raw = max / count;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const f = raw / pow;
  const step = (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * pow;
  const n = Math.ceil(max / step - 1e-9);
  return Array.from({ length: n + 1 }, (_, i) => Number((i * step).toPrecision(12)));
}

/** SVG points for a straight line over 0..w by 0..h, `top` being the value at the top edge. A flat zero line for no data. */
export function linePoints(values: number[], top: number, w: number, h: number): string {
  const v = values.filter((x) => Number.isFinite(x));
  if (v.length === 0) return `0,${h} ${w},${h}`;
  const y = (x: number) => h - (Math.max(x, 0) / (top > 0 ? top : 1)) * h;
  if (v.length === 1) return `0,${y(v[0] as number)} ${w},${y(v[0] as number)}`;
  return v.map((x, i) => `${(i / (v.length - 1)) * w},${y(x)}`).join(" ");
}

/**
 * A centred moving average over `span` points (odd). A latency read from histogram buckets steps
 * between a few values; the average shows the trend instead of a square wave. Edges use what is there.
 */
export function smooth(values: number[], span = 5): number[] {
  const half = Math.floor(span / 2);
  return values.map((v, i) => {
    if (!Number.isFinite(v)) return v;
    const near = values.slice(Math.max(0, i - half), i + half + 1).filter(Number.isFinite);
    return near.reduce((a, b) => a + b, 0) / near.length;
  });
}
