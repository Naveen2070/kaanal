// One request as a block: a header that opens and closes it (a real button, aria-expanded), and
// beneath it the hops in time order on a tone-step gutter. One status mark per request: a grey dot
// and "ok", or the failure's word in the negative colour. The total is right-aligned, tabular.

import { ChevronRight } from "lucide-react";
import { dur } from "../logFormat";
import { HopRow } from "./HopRow";
import type { RequestGroup } from "./groups";

export function RequestBlock({
  group: g,
  open,
  onToggle,
  depths,
}: {
  group: RequestGroup;
  open: boolean;
  onToggle: () => void;
  depths: Map<string, number>;
}) {
  // the visible parts run together without spaces, so the name is spelled out for a screen reader
  const label = [
    `r-${g.request}`,
    g.stream,
    g.failure ?? "ok",
    g.total_ms === null ? null : dur(g.total_ms),
    g.waited,
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <li data-group={g.key} className="border-t border-rule first:border-t-0">
      <button
        type="button"
        aria-expanded={open}
        aria-label={label}
        onClick={onToggle}
        className={`flex min-h-8 w-full items-start gap-2 px-3 py-1.5 text-left text-xs leading-5 hover:bg-raised focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent ${
          open ? "bg-active" : ""
        }`}
      >
        <ChevronRight
          size={14}
          aria-hidden
          className={`mt-[3px] flex-none text-muted transition-transform ${open ? "rotate-90" : ""}`}
        />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="flex-none font-medium text-text tabular-nums">r-{g.request}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted">{g.stream}</span>
          </span>
          {g.waited ? <span className="block text-muted">{g.waited}</span> : null}
        </span>
        {g.failure ? (
          <span className="flex flex-none items-center gap-1.5 whitespace-nowrap text-negative">
            <span aria-hidden className="size-1.5 rounded-full bg-bad-fill" />
            {g.failure}
          </span>
        ) : (
          <span className="flex flex-none items-center gap-1.5 whitespace-nowrap text-muted">
            <span aria-hidden className="size-1.5 rounded-full bg-faint" />
            ok
          </span>
        )}
        <span className="w-16 flex-none text-right text-text tabular-nums">
          {g.total_ms === null ? <span className="text-faint">–</span> : dur(g.total_ms)}
        </span>
      </button>
      {open ? (
        <ul className="m-0 list-none bg-well py-1 pl-2">
          {g.lines.map((l, i) => (
            <HopRow
              key={i}
              line={l}
              at={g.axis.rows[i] as RequestGroup["axis"]["rows"][number]}
              depths={depths}
            />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
