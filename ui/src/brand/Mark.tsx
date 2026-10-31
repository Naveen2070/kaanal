// The Kaanal mark: a half disc standing on a horizon, cut into bars by heat-haze gaps that grow toward
// the horizon, and its reflection as slices that shear sideways. Inline SVG in `currentColor`, so the
// caller picks the colour (brand blue, white on a tile, amber for marketing). Numbers: geometry.mjs.

import { useId } from "react";
import { DISC, FAVICON, GRID, HORIZON, SLICES, SUN_BARS, discPath } from "./geometry.mjs";

export type MarkVariant = "full" | "sun" | "favicon";

export function Mark({
  size = 24,
  variant = "full",
  className,
  title,
}: {
  size?: number;
  variant?: MarkVariant;
  className?: string;
  /** Without a title the mark is decoration and hidden from assistive technology. */
  title?: string;
}) {
  // useId gives ":r0:", which is legal in an id but awkward in url(#...): keep it plain
  const clipId = `kaanal-sun-${useId().replaceAll(":", "")}`;
  const favicon = variant === "favicon";
  const grid = favicon ? FAVICON.grid : GRID;
  const disc = favicon ? FAVICON.disc : DISC;
  const bars = favicon ? FAVICON.bars : SUN_BARS;
  const x = disc.cx - disc.r - 1.2;
  const width = 2 * disc.r + 2.4;
  const slices = favicon ? [FAVICON.slice] : SLICES;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${grid} ${grid}`}
      fill="currentColor"
      className={className}
      {...(title ? { role: "img" } : { "aria-hidden": true })}
    >
      {title ? <title>{title}</title> : null}
      <clipPath id={clipId}>
        <path d={discPath(disc)} />
      </clipPath>
      <g clipPath={`url(#${clipId})`}>
        {bars.map((b) => (
          <rect key={b.y} x={x} y={b.y} width={width} height={b.h} />
        ))}
      </g>
      {variant === "full" ? (
        <line
          x1={HORIZON.x1}
          x2={HORIZON.x2}
          y1={HORIZON.y}
          y2={HORIZON.y}
          stroke="currentColor"
          strokeWidth={HORIZON.width}
          opacity={HORIZON.opacity}
        />
      ) : null}
      {variant === "sun"
        ? null
        : slices.map((s) => (
            <rect
              key={s.y}
              x={s.x}
              y={s.y}
              width={s.w}
              height={s.h}
              rx={s.h / 2}
              opacity={s.opacity}
            />
          ))}
    </svg>
  );
}
