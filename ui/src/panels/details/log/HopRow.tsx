// One hop of a request: when, who (indented by how deep in the call graph), what kind, what
// happened, the wait-versus-work bar on the request's shared axis, and the time. A narrow rail drops
// the time stamp, then the bar and the words, never the badge or the ms (container queries, so the
// drawer and the rail each answer to their own width).

import type { LogLine } from "../../../data/api/types";
import { clock, eventWords, formatLine, hopDepth, hopMs, hopType, whoOf } from "../logFormat";
import type { RequestGroup } from "./groups";

/** 12px per level; a deep chain stops at five so a narrow rail still has room for the name. */
const STEP = 12;
const MAX_DEPTH = 5;

export function HopRow({
  line: l,
  at,
  depths,
}: {
  line: LogLine;
  at: RequestGroup["axis"]["rows"][number];
  depths: Map<string, number>;
}) {
  const type = hopType(l);
  const t = hopMs(l);
  const indent = Math.min(hopDepth(l, depths), MAX_DEPTH) * STEP;
  return (
    <li
      title={formatLine(l)}
      className="flex min-h-[30px] items-center gap-2 px-3 text-xs leading-4"
    >
      <span className="hidden w-[4.5rem] flex-none whitespace-nowrap text-muted tabular-nums @md:block">
        {clock(l.t_ms)}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-text" style={{ paddingLeft: indent }}>
        {whoOf(l)}
      </span>
      <span
        className={`flex-none whitespace-nowrap rounded border px-1.5 text-[11px] leading-4 ${
          type.tone === "bad" ? "border-negative text-negative" : "border-border text-muted"
        }`}
      >
        {type.label}
      </span>
      {/* the words stay in the page for a screen reader even where there is no room to show them */}
      <span className="sr-only @lg:not-sr-only @lg:w-52 @lg:flex-none @lg:truncate @lg:text-muted">
        {eventWords(l)}
      </span>
      <Bar at={at} line={l} />
      <span className="w-14 flex-none text-right text-text tabular-nums">
        {t === null ? <span className="text-faint">–</span> : ms(t)}
      </span>
    </li>
  );
}

function ms(t: number): string {
  return Number(t.toFixed(t < 10 ? 2 : 1)).toLocaleString("en-US");
}

/** Wait (warn fill) then work (ok fill) at the hop's place on the request's axis; a failure is a bad tick. */
function Bar({ at, line: l }: { at: RequestGroup["axis"]["rows"][number]; line: LogLine }) {
  const { left, wait, work } = at;
  const failure = l.wait_ms === null && l.service_ms === null;
  return (
    <span
      aria-hidden
      className="relative hidden h-2 w-10 flex-none rounded-sm bg-rule @min-[300px]:block @md:w-28"
    >
      {failure ? (
        <span
          className="absolute top-0 h-2 w-1 rounded-sm bg-bad-fill"
          style={{ left: `${Math.min(left, 96)}%` }}
        />
      ) : (
        <>
          {wait > 0 ? (
            <span
              className="absolute top-0 h-2 bg-warn-fill"
              style={{ left: `${left}%`, width: `${wait}%` }}
            />
          ) : null}
          <span
            className="absolute top-0 h-2 bg-ok-fill"
            style={{ left: `${left + wait}%`, width: `${Math.max(work, 1)}%` }}
          />
        </>
      )}
    </span>
  );
}
