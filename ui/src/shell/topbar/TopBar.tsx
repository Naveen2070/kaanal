// The top-left card, stacked: the title (mark and project) with undo and redo; Run, Apply and Save in one row; then
// what the loop is doing over the draft's state. Undo and redo sit on the title row.
// Theme and help are a small sheet of their own at the top right (TopTools). Why a run could not
// give numbers is a notice (a toast), not a line in the bar. The key figures and the offered load
// are the strip under it (KpiStrip.tsx).

import { BookOpen, CircleHelp, Play } from "lucide-react";
import { Mark } from "../../look/brand/Mark";
import { useFirstRun } from "../../overlays/first-run/store";
import { useGuide } from "../../overlays/guide/store";
import { MenuButton } from "../../overlays/menu/MenuButton";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { SaveFlow } from "./SaveFlow";
import { ThemeToggle } from "./ThemeToggle";
import { UndoRedo } from "./UndoRedo";

export function TopBar({ onRun }: { onRun: () => void }) {
  const view = useDraft((st) => st.view);
  const status = useRun((s) => s.status);

  const line =
    status === "running"
      ? "Running..."
      : status === "blocked"
        ? "Not up to date"
        : status === "done"
          ? "Up to date"
          : "Edits re-run on their own";

  return (
    <header className="flex w-76 flex-col gap-2 p-3 text-sm">
      <div className="flex items-center justify-between gap-2">
        <h1 className="m-0 flex min-w-0 items-center gap-2 text-base font-semibold">
          <Mark size={24} className="shrink-0 text-accent" />
          <span className="shrink-0">Kaanal </span>
          {view?.project_name ? (
            <span
              className="truncate font-normal text-muted"
              title="The project this Studio is editing"
            >
              / {view.project_name}
            </span>
          ) : null}
        </h1>
        <UndoRedo />
      </div>
      <SaveFlow
        lead={
          <button
            type="button"
            data-tour="run"
            onClick={onRun}
            className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            <Play size={14} aria-hidden />
            Run
          </button>
        }
        note={
          <output className={`text-xs ${status === "blocked" ? "text-warn" : "text-muted"}`}>
            {line}
          </output>
        }
      />
    </header>
  );
}

/** Theme and help: the part of the top bar that is about the app, not the draft. */
export function TopTools() {
  const replayTour = useFirstRun((st) => st.replayTour);
  return (
    <div className="flex items-center gap-1 p-1.5">
      <ThemeToggle />
      <button
        type="button"
        aria-label="Glossary"
        title="Glossary: what every part and figure means"
        onClick={() => useGuide.getState().show()}
        className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <BookOpen size={16} aria-hidden />
      </button>
      <MenuButton />
      <button
        type="button"
        aria-label="Show the tour"
        title="Show the tour"
        onClick={replayTour}
        className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <CircleHelp size={16} aria-hidden />
      </button>
    </div>
  );
}
