// What the hop bars and the time column mean, in the colours the bars already use (warn fill = wait,
// ok fill = work, bad fill = a failure). A network hop is drawn as work, so the entry says so.

export function Legend() {
  const swatch = "block h-2 flex-none rounded-sm";
  return (
    <ul
      aria-label="What the bar colours mean"
      className="m-0 mb-1.5 flex list-none flex-wrap gap-x-3.5 gap-y-1 p-0 text-[11px] text-muted"
    >
      <li className="flex items-center gap-1.5">
        <span aria-hidden className={`${swatch} w-3.5 bg-warn-fill`} />
        Waiting in line
      </li>
      <li className="flex items-center gap-1.5">
        <span aria-hidden className={`${swatch} w-3.5 bg-ok-fill`} />
        Working or on the wire
      </li>
      <li className="flex items-center gap-1.5">
        <span aria-hidden className={`${swatch} w-1 bg-bad-fill`} />
        Failed here
      </li>
    </ul>
  );
}

/** The column names above the request rows; the timeline and the clock drop out on a narrow rail like their rows do. */
export function ColumnHeads() {
  return (
    <div
      aria-hidden
      className="flex gap-2 border-b border-rule bg-sheet py-1 pr-3 pl-5 text-[11px] text-muted"
    >
      <span className="min-w-0 flex-1">Request, then each hop</span>
      <span className="hidden w-10 flex-none @min-[300px]:block @md:w-28">Timeline</span>
      <span className="w-16 flex-none text-right">Time</span>
    </div>
  );
}
