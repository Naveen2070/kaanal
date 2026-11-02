// The meter (spec 4): an 8px track filled left to right at the true ratio, green below 70%, amber
// from 70%, red from 90%, with the words above it in ink (never white on a fill). Colour only
// repeats what the words say. A `stale` figure is marked in words until the next run lands.

export type Tone = "ok" | "warn" | "bad";

export function meterTone(ratio: number): Tone {
  return ratio >= 0.9 ? "bad" : ratio >= 0.7 ? "warn" : "ok";
}

const FILL: Record<Tone, string> = {
  ok: "bg-ok-fill",
  warn: "bg-warn-fill",
  bad: "bg-bad-fill",
};

export function Meter({
  ratio,
  label,
  stale = false,
}: {
  /** 0 to 1 (more is clamped); null when there is nothing to measure. */
  ratio: number | null;
  label: string;
  stale?: boolean;
}) {
  const pct = ratio === null ? 0 : Math.round(Math.min(Math.max(ratio, 0), 1) * 100);
  const bar = (
    <div aria-hidden className="h-2 overflow-hidden rounded-full bg-well">
      {ratio !== null && ratio > 0 ? (
        <div
          data-fill
          className={`h-full rounded-full transition-[width] duration-300 motion-reduce:transition-none ${FILL[meterTone(ratio)]}`}
          style={{ width: `${Math.min(ratio, 1) * 100}%` }}
        />
      ) : null}
    </div>
  );
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-text">{label}</span>
        {/* always laid out, so "updating" never shifts the row */}
        <span aria-hidden={!stale} className={stale ? "text-muted" : "invisible"}>
          updating
        </span>
      </div>
      {bar}
      {ratio === null ? null : (
        // the bar is for the eye; a native meter gives a screen reader the value and the words
        <meter
          className="sr-only"
          aria-label="Busy"
          aria-valuetext={label}
          min={0}
          max={100}
          value={pct}
        />
      )}
    </div>
  );
}
