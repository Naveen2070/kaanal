import { useId } from "react";

/**
 * Kaanal mirage-sun mark (matching ui/src/look/brand/geometry.mjs).
 * Half disc cut by heat-haze gaps on a horizon, with sheared reflection slices.
 */
export function KaanalMark({
  size = 24,
  className,
  title,
}: {
  size?: number;
  className?: string;
  title?: string;
}) {
  const clipId = `kaanal-sun-${useId().replaceAll(":", "")}`;

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      className={className}
      {...(title ? { role: "img" } : { "aria-hidden": true })}
    >
      {title ? <title>{title}</title> : null}
      <clipPath id={clipId}>
        <path d="M3.2 13.5A8.8 8.8 0 0 1 20.8 13.5Z" />
      </clipPath>
      <g clipPath={`url(#${clipId})`}>
        <rect x="2" y="3.9" width="20" height="3.7" />
        <rect x="2" y="8.5" width="20" height="2.4" />
        <rect x="2" y="12.4" width="20" height="1.1" />
      </g>
      <line x1="1" x2="23" y1="14.9" y2="14.9" stroke="currentColor" strokeWidth="1.1" opacity="0.55" />
      <rect x="4.6" y="16.3" width="14.8" height="1.8" rx="0.9" opacity="0.7" />
      <rect x="7.4" y="19" width="11.2" height="1.6" rx="0.8" opacity="0.45" />
      <rect x="3.2" y="21.5" width="7.6" height="1.3" rx="0.65" opacity="0.26" />
    </svg>
  );
}
