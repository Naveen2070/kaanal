// The top bar, in groups with a divider between them: the title (mark and project), Run with a plain
// line saying what the loop is doing, undo and redo, then the draft's state with Apply and Save.
// Theme and help are a small sheet of their own at the top right (TopTools). Why a run could not
// give numbers is a notice (a toast), not a line in the bar. The key figures and the offered load
// are the strip under it (KpiStrip.tsx).

import { CircleHelp, Play } from "lucide-react";
import { Mark } from "../brand/Mark";
import { useFirstRun } from "../first-run/store";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { SaveFlow } from "./SaveFlow";
import { ThemeToggle } from "./ThemeToggle";
import { UndoRedo } from "./UndoRedo";

const Divider = () => <span aria-hidden className="hidden h-6 w-px bg-border sm:block" />;

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
    <header className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-2 text-sm">
      <h1 className="m-0 flex items-center gap-2 text-base font-semibold">
        <Mark size={24} className="text-accent" />
        {view?.project_name ?? "Kaanal Studio"}
      </h1>
      <Divider />
      <div className="flex items-center gap-2.5">
        <button
          type="button"
          data-tour="run"
          onClick={onRun}
          className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          <Play size={14} aria-hidden />
          Run
        </button>
        <output className={`min-w-28 text-xs ${status === "blocked" ? "text-warn" : "text-muted"}`}>
          {line}
        </output>
      </div>
      <Divider />
      <UndoRedo />
      <Divider />
      <SaveFlow />
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
