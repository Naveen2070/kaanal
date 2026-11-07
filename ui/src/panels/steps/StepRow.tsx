// A list of steps and the row of each (Part 9). One recursive piece: the route's chain is a list, and a
// `parallel:` step carries a list per branch. A row is one line with a button that opens its fields, and
// buttons to move it up or down among its neighbours and to delete it. Every action is one patch.

import { ArrowDown, ArrowUp, ChevronRight, Trash2 } from "lucide-react";
import { useContext, useState } from "react";
import { useDraft } from "../../data/store/draft";
import { AddPick } from "./AddStep";
import { CacheFields } from "./CacheFields";
import { CallFields } from "./CallFields";
import { stepLabel } from "./describe";
import { Announce, focusRow, moveKey, pathKey, siblingPaths } from "./keys";
import {
  addBranchOps,
  addToBranchOps,
  moveOps,
  removeBranchOps,
  removeStepOps,
  type Branch,
  type ParallelStep,
  type Step,
} from "./model";

export interface Pickable {
  callable: string[];
  caches: string[];
}

const BTN =
  "flex size-6 shrink-0 items-center justify-center rounded-md border border-control text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40";
const TEXT_BTN =
  "rounded-md border border-control px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40";

/** The steps of one list (a route's chain, or one branch), each a row that moves among its neighbours. */
export function StepList({ steps, label, pick }: { steps: Step[]; label: string; pick: Pickable }) {
  const patch = useDraft((s) => s.patch);
  const say = useContext(Announce);
  return (
    <ol aria-label={label} className="m-0 mb-2 list-none p-0">
      {steps.map((s, i) => (
        <Row
          key={i}
          step={s}
          pick={pick}
          index={i}
          count={steps.length}
          onMove={(dir) => {
            const ops = moveOps(steps, i, dir);
            if (ops.length === 0) return;
            void patch(ops);
            say(`Moved ${stepLabel(s)} to position ${i + dir + 1} of ${steps.length}`);
            focusRow([steps[i + dir]?.path ?? s.path]);
          }}
          onRemove={() => {
            void patch(removeStepOps(s));
            say(`Deleted ${stepLabel(s)}`);
            focusRow(siblingPaths(s.path), () =>
              document.querySelector<HTMLElement>("[data-steps-add]"),
            );
          }}
        />
      ))}
    </ol>
  );
}

function Row({
  step,
  pick,
  index,
  count,
  onMove,
  onRemove,
}: {
  step: Step;
  pick: Pickable;
  index: number;
  count: number;
  onMove: (dir: -1 | 1) => void;
  onRemove: () => void;
}) {
  const label = stepLabel(step);
  const [open, setOpen] = useState(false);
  const editable = step.kind === "call" || step.kind === "cache";
  return (
    <li
      data-step-path={pathKey(step.path)}
      className="mb-1 rounded-md border border-border bg-raised/50"
    >
      {/* Alt+Up and Alt+Down from any button of the row move it; the buttons do the same by click */}
      <div
        role="presentation"
        onKeyDown={(e) => {
          const dir = moveKey(e);
          if (dir === 0) return;
          e.preventDefault();
          onMove(dir);
        }}
        className="flex items-center gap-1.5 px-2 py-1"
      >
        {editable ? (
          <button
            type="button"
            aria-expanded={open}
            aria-label={`Edit ${label}`}
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
            data-row-focus
            onClick={() => setOpen(!open)}
            className="flex min-w-0 flex-1 items-center gap-1 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-accent"
          >
            <ChevronRight
              size={12}
              aria-hidden
              className={`shrink-0 text-muted ${open ? "rotate-90" : ""}`}
            />
            <span className="min-w-0 flex-1 truncate font-mono text-xs" title={label}>
              {label}
            </span>
          </button>
        ) : (
          <span className="min-w-0 flex-1 truncate font-mono text-xs" title={label}>
            {label}
          </span>
        )}
        {step.kind === "call" && step.retry !== undefined ? (
          <span className="shrink-0 rounded-full bg-accent/15 px-1.5 text-[11px]">Retry</span>
        ) : null}
        <button
          type="button"
          aria-label={`Move ${label} up`}
          disabled={index === 0}
          onClick={() => onMove(-1)}
          className={BTN}
        >
          <ArrowUp size={12} aria-hidden />
        </button>
        <button
          type="button"
          aria-label={`Move ${label} down`}
          disabled={index === count - 1}
          onClick={() => onMove(1)}
          className={BTN}
        >
          <ArrowDown size={12} aria-hidden />
        </button>
        <button
          type="button"
          aria-label={`Delete ${label}`}
          data-row-focus={editable ? undefined : true}
          onClick={onRemove}
          className={BTN}
        >
          <Trash2 size={12} aria-hidden />
        </button>
      </div>
      {step.kind === "call" && open ? <CallFields step={step} callable={pick.callable} /> : null}
      {step.kind === "cache" && open ? <CacheFields step={step} caches={pick.caches} /> : null}
      {step.kind === "parallel" ? <Parallel step={step} pick={pick} /> : null}
      {step.kind === "locked" ? <Locked raw={step.raw} /> : null}
    </li>
  );
}

/** The branches of a `parallel:` block: each its own list, with add and remove. They run together. */
function Parallel({ step, pick }: { step: ParallelStep; pick: Pickable }) {
  const patch = useDraft((s) => s.patch);
  return (
    <div className="px-2 pb-1 pl-4">
      <p className="mb-1 text-[11px] text-muted">
        The branches start together; the block ends with the slowest.
      </p>
      {step.branches.map((b, i) => (
        <BranchBox
          key={i}
          branch={b}
          n={i + 1}
          pick={pick}
          onRemove={
            step.branches.length > 1 ? () => void patch(removeBranchOps(step, i)) : undefined
          }
          onAdd={(node) => void patch(addToBranchOps(b, node))}
        />
      ))}
      <AddPick
        options={pick.callable}
        selectLabel="Component for a new branch"
        button="Add branch"
        className="mb-1"
        onAdd={(node) => void patch(addBranchOps(step, node))}
      />
    </div>
  );
}

function BranchBox({
  branch,
  n,
  pick,
  onRemove,
  onAdd,
}: {
  branch: Branch;
  n: number;
  pick: Pickable;
  onRemove?: () => void;
  onAdd: (node: string) => void;
}) {
  return (
    <fieldset className="m-0 mb-1.5 min-w-0 rounded-md border border-border p-1.5">
      <legend className="px-1 text-[11px] text-muted">Branch {n}</legend>
      <StepList steps={branch.steps} label={`Steps of branch ${n}`} pick={pick} />
      <div className="flex flex-wrap items-center gap-1.5">
        <AddPick
          options={pick.callable}
          selectLabel={`Component to call in branch ${n}`}
          button={`Add call to branch ${n}`}
          onAdd={onAdd}
        />
        {onRemove ? (
          <button type="button" onClick={onRemove} className={TEXT_BTN}>
            Remove branch {n}
          </button>
        ) : null}
      </div>
    </fieldset>
  );
}

/** A step the loader would not accept (no `call`, `cache` or `parallel`, or a stray key beside one): shown as written, so it can be found and deleted or moved. */
function Locked({ raw }: { raw: unknown }) {
  const text = JSON.stringify(raw) ?? String(raw);
  return (
    <div className="px-2 pb-1.5 text-[11px] text-muted">
      <p className="m-0 mb-0.5">
        This is not a step Kaanal can run, so the draft will not start until it is changed in the
        file or deleted here. It is shown as written:
      </p>
      <code className="block truncate font-mono" title={text}>
        {text}
      </code>
    </div>
  );
}
