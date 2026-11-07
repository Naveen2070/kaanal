// One folding group of Controls boxes. Closed, its header says in one line what is set ("3 tries ·
// 50 ms x2"), so a power user reads every group without opening it; open, it shows the boxes.
// A dot says the group holds a setting the person changed, and Reset puts the group back. Open or
// closed is remembered (`store.ts`). The look is `Section`'s: one hairline under it, the same
// heading size, nothing new.

import { ChevronRight } from "lucide-react";
import { useId } from "react";
import { useGroupOpen, useGroups } from "./store";

/** The closed summary as small pills (parts split on " · "); tinted when the group holds a change. */
function Pills({ summary, changed }: { summary: string; changed: boolean }) {
  const parts = summary.split(" · ");
  return (
    <span className="ml-auto flex min-w-0 gap-1 overflow-hidden" title={summary}>
      {parts.map((p, i) => (
        <span
          key={i}
          className={`min-w-0 truncate rounded-full px-1.5 text-[11px] font-normal ${
            changed ? "bg-accent/15 text-text" : "bg-raised text-muted"
          }`}
        >
          {p}
          {i < parts.length - 1 ? <span className="sr-only"> · </span> : null}
        </span>
      ))}
    </span>
  );
}

export function Group({
  id,
  title,
  summary,
  changed = false,
  onReset,
  defaultOpen = false,
  children,
}: {
  /** Stable and unique, e.g. "wire.retry": the key the open state is kept under. */
  id: string;
  title: string;
  /** What is set, when the group is closed. */
  summary?: string;
  /** The group holds a setting that differs from the default. */
  changed?: boolean;
  /** Puts every setting in the group back (one undo step). Offered only when `changed`. */
  onReset?: () => void;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const bodyId = useId();
  const open = useGroupOpen(id, defaultOpen);
  return (
    <section className="mb-2 border-b pb-2" data-group={id}>
      <h3 className="m-0 text-[13px] font-semibold text-text">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => useGroups.getState().toggle(id, defaultOpen)}
          className="flex min-h-9 w-full items-center gap-1.5 rounded-sm text-left hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <ChevronRight
            size={14}
            aria-hidden
            className={`shrink-0 text-muted ${open ? "rotate-90" : ""}`}
          />
          <span className="shrink-0">{title}</span>
          {changed ? (
            <>
              <span aria-hidden className="size-1.5 shrink-0 rounded-full bg-accent" />
              <span className="sr-only"> (changed from the default)</span>
            </>
          ) : null}
          {!open && summary ? <Pills summary={summary} changed={changed} /> : null}
        </button>
      </h3>
      {open ? (
        <div id={bodyId} className="pt-1">
          {children}
          {changed && onReset ? (
            <button
              type="button"
              aria-label={`Reset ${title}`}
              onClick={onReset}
              className="min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              Reset to default
            </button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
