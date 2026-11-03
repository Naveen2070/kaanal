// Placeholders for the lazy tabs and for a first run in flight: the final layout with grey bars at
// real size, no shimmer, so nothing moves when the content arrives (spec 6.2). First-load code, so
// it stays tiny; the sizes are shared with the real thing through these constants.

/** One row of "What breaks first". */
export const RANK_ROW = "h-11";
/** The latency heatmap's box (60 windows by 20 buckets). */
export const HEAT_ASPECT = "aspect-[3/1]";

const Bar = ({ className }: { className: string }) => (
  <div aria-hidden className={`rounded bg-well ${className}`} />
);

export function ResultsSkeleton() {
  return (
    <output aria-label="Loading the results" className="block p-3">
      <Bar className="mb-3 h-4 w-2/3" />
      <div className="mb-4 space-y-1">
        {[0, 1, 2].map((i) => (
          <Bar key={i} className={RANK_ROW} />
        ))}
      </div>
      <Bar className={`w-full ${HEAT_ASPECT}`} />
    </output>
  );
}

/** The Log tab's frame while its code loads. */
export function LogSkeleton() {
  return (
    <output aria-label="Loading the log" className="block p-3">
      <Bar className="mb-2 h-4 w-2/3" />
      <Bar className="min-h-40 w-full" />
    </output>
  );
}

/** The Live tab's frame while its code loads. */
export function LiveSkeleton() {
  return (
    <output aria-label="Loading the live controls" className="block p-3">
      <Bar className="mb-2 h-4 w-2/3" />
      <Bar className="mb-3 h-16 w-full" />
      <Bar className="h-24 w-full" />
    </output>
  );
}
