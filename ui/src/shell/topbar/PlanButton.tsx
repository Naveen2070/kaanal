// The Plan button in the top bar (step 9c-3): opens the plan dialog, in every mode (a plan is of the
// applied design, whatever the live stream is doing; the dialog says so). The marker says in words
// what the plan on screen is doing, so closing the dialog never hides a running or finished plan.

import { Gauge } from "lucide-react";
import { useDraft } from "../../data/store/draft";
import { usePlan } from "../../data/store/plan";
import { isStale } from "../../panels/plan/planMath";

const MARK = {
  running: "running",
  done: "done",
  earlier: "earlier design",
  failed: "failed",
} as const;

/** What the plan on screen is doing, in words, or null when there is none. */
function useMark(): string | null {
  const status = usePlan((s) => s.status);
  const record = usePlan((s) => s.record);
  const sent = usePlan((s) => s.sent);
  const view = useDraft((s) => s.view);
  const stale = record && view ? isStale(record, view, sent?.boot ?? null) : null;
  if (status === "running") return MARK.running;
  if (status === "failed") return MARK.failed;
  if (status === "done") return stale ? MARK.earlier : MARK.done;
  return null;
}

/** The button, small enough to sit on the mode switch's row. */
export function PlanButton() {
  return (
    <button
      type="button"
      aria-haspopup="dialog"
      title="Plan a size: what the applied design needs to hold a target load"
      onClick={() => usePlan.getState().setOpen(true)}
      className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-control bg-sheet px-3 text-sm font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
    >
      <Gauge size={14} aria-hidden />
      Plan
    </button>
  );
}

/** The marker, as its own line under the row (a long word such as "earlier design" does not fit beside the button). */
export function PlanMark() {
  const mark = useMark();
  return mark ? <output className="block text-xs text-muted">{`Plan: ${mark}`}</output> : null;
}
