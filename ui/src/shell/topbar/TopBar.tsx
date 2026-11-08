// The top-left card, stacked: the title (mark and project) with undo and redo; Run, Apply and Save in one row; then
// what the loop is doing over the draft's state. Undo and redo sit on the title row.
// Theme and help are a small sheet of their own at the top right (TopTools). Why a run could not
// give numbers is a notice (a toast), not a line in the bar. The key figures and the offered load
// are the strip under it (KpiStrip.tsx).

import { Play } from "lucide-react";
import { Mark } from "../../look/brand/Mark";
import { MenuButton } from "../../overlays/menu/MenuButton";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { useMode, type Mode } from "../../data/store/mode";
import { useRun } from "../../data/store/run";
import { SaveFlow } from "./SaveFlow";
import { ThemeToggle } from "./ThemeToggle";
import { UndoRedo } from "./UndoRedo";

const MODES: { id: Mode; label: string; hint: string }[] = [
  { id: "live", label: "Live", hint: "Traffic runs all the time; changes show at once" },
  { id: "batch", label: "Batch", hint: "Run once, seeded and repeatable, then compare" },
];

/** Live or batch: a two-way switch beside Run. */
function ModeSwitch() {
  const mode = useMode((s) => s.mode);
  const set = useMode((s) => s.set);
  return (
    <fieldset
      aria-label="Mode"
      className="m-0 flex rounded-lg border border-control bg-sheet p-0.5 min-w-0 border"
    >
      {MODES.map((m) => (
        <button
          key={m.id}
          type="button"
          aria-pressed={mode === m.id}
          title={m.hint}
          onClick={() => set(m.id)}
          className={`h-7 rounded-md px-2.5 text-xs font-medium focus-visible:outline-2 focus-visible:outline-accent ${mode === m.id ? "bg-raised text-text" : "text-muted hover:bg-raised"}`}
        >
          {m.label}
        </button>
      ))}
    </fieldset>
  );
}

export function TopBar({ onRun }: { onRun: () => void }) {
  const view = useDraft((st) => st.view);
  const status = useRun((s) => s.status);
  const mode = useMode((s) => s.mode);
  const pendingStructure = useLive((s) => {
    const e = s.lastEdit;
    return e !== null && "structure_pending" in e && e.structure_pending;
  });

  return (
    <header className="flex w-76 flex-col gap-2 p-3 text-sm">
      <div className="flex items-center justify-between gap-2">
        <h1 className="m-0 flex min-w-0 items-center gap-2 text-base font-semibold">
          <Mark size={24} className="shrink-0 text-accent" />
          <span className="shrink-0">Kaanal </span>
          {view?.project_name ? (
            <span
              className="truncate font-normal text-muted"
              title={`${view.project_name}: the project this Studio is editing`}
            >
              / {view.project_name}
            </span>
          ) : null}
        </h1>
        <UndoRedo />
      </div>
      <ModeSwitch />
      {mode === "live" && pendingStructure ? (
        <output className="block text-xs text-muted">
          Parameter edits run at once. Added or removed components and wires wait for Apply, which
          restarts the live run.
        </output>
      ) : null}
      <SaveFlow
        lead={
          mode === "batch" ? (
            <button
              type="button"
              data-tour="run"
              onClick={onRun}
              className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              <Play size={14} aria-hidden />
              Run
            </button>
          ) : null
        }
        run={status}
      />
    </header>
  );
}

/** Theme and the menu (glossary, tour, profile): the part of the top bar that is about the app, not the draft. */
export function TopTools() {
  return (
    <div className="flex items-center gap-1 p-1.5">
      <ThemeToggle />
      <MenuButton />
    </div>
  );
}
