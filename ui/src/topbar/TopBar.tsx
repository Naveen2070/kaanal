// The top bar: the heading (mark and project), a Run button, a plain line saying what the loop is
// doing, the draft state with Apply and Save, the theme and the tour. The key figures and the offered
// load are the KPI strip under it (KpiStrip.tsx).

import { Mark } from "../brand/Mark";
import { useFirstRun } from "../first-run/store";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { SaveFlow } from "./SaveFlow";
import { ThemeToggle } from "./ThemeToggle";

export function TopBar({ onRun }: { onRun: () => void }) {
  const view = useDraft((st) => st.view);
  const replayTour = useFirstRun((st) => st.replayTour);
  const { status, lastError } = useRun();

  const line =
    status === "running"
      ? "Running..."
      : lastError
        ? lastError
        : status === "done"
          ? "Up to date"
          : "Edit anything: it re-runs after a short pause.";

  return (
    <header className="flex min-h-11 flex-wrap items-center gap-4 px-4 py-1 text-sm">
      <h1 className="flex items-center gap-2 text-base font-semibold">
        <Mark size={24} className="text-accent" />
        {view?.project_name ?? "Kaanal Studio"}
      </h1>
      <button
        type="button"
        data-tour="run"
        onClick={onRun}
        className="h-8 rounded-md border border-control bg-sheet px-3 hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
      >
        Run
      </button>
      <output className={`text-xs ${lastError ? "text-negative" : "text-muted"}`}>{line}</output>
      <SaveFlow />
      <div className="ml-auto flex items-center gap-1">
        <ThemeToggle />
        <button
          type="button"
          aria-label="Show the tour"
          title="Show the tour"
          onClick={replayTour}
          className="h-8 min-w-8 rounded-full border border-control px-2"
        >
          ?
        </button>
      </div>
    </header>
  );
}
