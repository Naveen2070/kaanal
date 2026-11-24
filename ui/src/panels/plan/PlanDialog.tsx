// The plan dialog (step 9c-3): a native modal over the canvas with the form, the progress while a plan
// runs, the result and its foot (where it was made, the report, Load proposed draft). Everything shown
// comes from the plan store, which outlives the dialog: closing it neither stops nor loses a plan.
// A plan sizes the applied design, so Live and Admin mode (no steady load) cannot start one, and a
// busy simulator is a plain note, never an error.

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { PlanDoc } from "../../data/api/types.plan";
import { useDraft } from "../../data/store/draft";
import { draftBody } from "../../data/store/loop";
import { isLive, useMode } from "../../data/store/mode";
import { usePlan } from "../../data/store/plan";
import { useRun } from "../../data/store/run";
import { useSweep } from "../../data/store/sweep";
import { CopySaveButtons } from "../details/ResultsExtras";
import { PlanForm } from "./PlanForm";
import { PlanLoad } from "./PlanLoad";
import { PlanResult } from "./PlanResult";
import { span } from "./PlanSearch";
import { PlanSteps } from "./PlanSteps";
import { errorWords, isStale, n1 } from "./planMath";

const BUSY = "The simulator is busy with another run, sweep or plan. Start again when it ends.";
const STALE = {
  restarted: "Made before the server restarted. Start a new plan to see the design in force.",
  earlier:
    "Made on an earlier design: the design was applied again since. Start a new plan to see the design in force.",
};

const close = () => usePlan.getState().setOpen(false);

/** What the plan on screen was asked for, from the record (the form may hold other values by now). */
function asked(r: PlanDoc["request"]): string {
  const target =
    r.target.req_s !== undefined
      ? `${n1(r.req_s)} req/s`
      : `${n1(r.factor)}x the profile's rates (${n1(r.req_s)} req/s)`;
  return `${target}, p99 ${n1(r.limit.p99_ms)} ms, errors ${errorWords(r.limit.error_rate)}`;
}

export function PlanDialog() {
  const box = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const noteId = useId();
  const { status, record, report, error, sent, form, setForm, start } = usePlan();
  const live = isLive(useMode((m) => m.mode));
  const view = useDraft((s) => s.view);
  const runBusy = useRun((s) => s.status === "running");
  const sweepBusy = useSweep((s) => s.status === "running");
  const elsewhere = runBusy || sweepBusy;
  const running = status === "running";
  const [now, setNow] = useState(() => Date.now());

  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    // when the dialog goes, focus goes back to the button that opened it
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);

  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);

  // the server plans the applied design with its first profile; only a clean draft is it
  const noProfile = view !== null && !view.dirty && draftBody(view) === null;
  const note =
    view === null
      ? "Loading the design; Start is ready once it arrives."
      : noProfile
        ? "The design has no traffic profile to plan from."
        : elsewhere && !running
          ? BUSY
          : null;

  const plan = status === "done" ? record?.plan : null;
  const stale = record && view ? isStale(record, view, sent?.boot ?? null) : null;
  // ponytail: elapsed from the server's start stamp; assumes the browser and server clocks agree (local server)
  // the server leaves `plan_steps` out until the first step is taken
  const taken = record?.plan_steps ?? [];
  const steps = taken.length;
  const elapsed = record ? Math.max(0, now - record.started_ms) : 0;

  return (
    <dialog
      ref={box}
      aria-labelledby={titleId}
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      className="max-h-[88vh] w-[min(60rem,calc(100vw-2rem))] overflow-y-auto rounded-2xl border border-border bg-sheet p-0 text-sm text-text shadow-float backdrop:bg-black/40"
    >
      <div className="flex items-start gap-3 border-b border-border px-5 py-4">
        <div className="min-w-0 flex-1">
          <h2 id={titleId} className="m-0 text-base font-semibold">
            Sizing plan
          </h2>
          <p className="m-0 mt-1 text-xs text-muted">
            Sizes the applied design to hold a target load. A relative what-if; prices are
            estimates.
          </p>
        </div>
        <button
          type="button"
          onClick={close}
          className="h-8 rounded-lg border border-control px-3 text-xs font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          Close
        </button>
      </div>

      <div className="space-y-2 px-5 py-4">
        <PlanForm
          value={form}
          onChange={(next) => setForm(next(usePlan.getState().form))}
          disabledWhy={note ? noteId : null}
          running={running}
          onStart={(body) => void start(body, view?.boot ?? null)}
        />
        {note ? (
          <p id={noteId} className="m-0 text-xs text-muted">
            {note}
          </p>
        ) : null}
        {/* in Live the draft is always "changed"; the one thing to know is what a plan is of */}
        {live ? (
          <p className="m-0 text-xs text-muted">
            Plans the applied design, not the live numbers; a change not applied yet is not in it.
          </p>
        ) : view?.dirty ? (
          <p className="m-0 text-xs text-muted">
            Plans the applied design; Apply to include your edits.
          </p>
        ) : null}
        {/* the slot is taken: a state to wait out, not a failure */}
        {status === "busy" && note !== BUSY ? (
          <p className="m-0 text-xs text-muted">{error ?? BUSY}</p>
        ) : null}
        {/* the live region changes only when a step arrives; the clock sits outside it */}
        <output aria-live="polite" className="block text-xs text-muted">
          {running ? `Planning: ${steps} ${steps === 1 ? "step" : "steps"} so far` : ""}
        </output>
        {running ? <p className="m-0 text-xs text-muted">{`${span(elapsed)} so far`}</p> : null}
        {running && taken.length > 0 ? <PlanSteps steps={taken} /> : null}
        {status === "done" && !plan ? (
          <p className="m-0 text-xs text-muted">No plan came back from the server.</p>
        ) : null}
        {status === "failed" ? (
          <p role="alert" className="m-0 text-xs text-negative">
            {`The plan failed: ${error ?? "no reason given"}`}
          </p>
        ) : null}
      </div>

      {plan && record ? (
        <>
          <div className="border-t border-border px-5 py-4">
            <PlanResult plan={plan} />
          </div>
          <div className="space-y-2 border-t border-border px-5 py-3 text-xs">
            <p className="m-0">{`Made on the applied design (generation ${record.generation}) for ${asked(plan.request)}`}</p>
            {stale ? <p className="m-0 text-warn">{STALE[stale]}</p> : null}
            {report !== null ? (
              <CopySaveButtons text={() => report} fileName={`kaanal-plan-${record.id}.md`} />
            ) : (
              <p className="m-0 text-muted">{`The report could not be fetched: ${error ?? "no reason given"}`}</p>
            )}
            {record.proposed_draft ? (
              <PlanLoad draft={record.proposed_draft} stale={stale !== null} />
            ) : null}
          </div>
        </>
      ) : null}
    </dialog>
  );
}
