// The starters shown on an empty canvas: pick one and its YAML replaces the (empty) draft. A card has a
// sketch of the system, its name, a line on what it is and a line on what to try (cut short; the
// round button beside it opens the whole line). Two columns when the box is wide enough (by the
// box's own width, not the window's), one otherwise; the list scrolls inside its box, which the
// canvas keeps between the top bar and the toolbar. A refusal is shown under the list; nothing is
// lost, the draft was empty.

import { Info } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useDraft } from "../data/store/draft";
import { StarterSketch } from "./StarterSketch";
import { STARTERS } from "./starters";

export function StarterList({ action }: { action?: ReactNode }) {
  const replaceText = useDraft((s) => s.replaceText);
  const [busy, setBusy] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  /** The card whose whole "Try" line is open. */
  const [open, setOpen] = useState<string | null>(null);
  const load = async (id: string) => {
    const starter = STARTERS.find((s) => s.id === id);
    if (!starter) return;
    setBusy(id);
    setProblem(null);
    try {
      const ok = await replaceText(await starter.text());
      if (!ok) setProblem(useDraft.getState().error ?? "That starter was refused.");
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="pointer-events-auto @container flex max-h-full min-h-0 w-[min(46rem,100%)] flex-col rounded-2xl border border-border bg-sheet p-3 text-sm shadow-float">
      <div className="mb-2.5 flex shrink-0 items-center gap-2.5">
        <h2 className="m-0 flex-1 text-sm font-semibold">Start from a system</h2>
        {action}
      </div>
      <ul className="m-0 grid min-h-0 list-none grid-cols-1 auto-rows-fr gap-2 overflow-y-auto p-0.5 @min-[35rem]:grid-cols-2">
        {STARTERS.map((s) => {
          const more = open === s.id;
          return (
            <li key={s.id} className="relative min-w-0">
              <button
                type="button"
                disabled={busy !== null}
                onClick={() => void load(s.id)}
                className="flex h-full w-full min-w-0 items-center gap-3 rounded-xl border border-border bg-card p-2.5 pr-9 text-left hover:bg-raised disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-accent"
              >
                <StarterSketch graph={s.graph} />
                <span className="min-w-0 flex-1">
                  <span className="block font-semibold">{s.title}</span>
                  <span className="line-clamp-2 text-xs text-muted">{s.blurb}</span>
                  <span className={`mt-0.5 block text-xs text-muted ${more ? "" : "truncate"}`}>
                    Try: {s.tryThis}
                  </span>
                </span>
              </button>
              <button
                type="button"
                aria-label={`About ${s.title}`}
                aria-expanded={more}
                onClick={() => setOpen(more ? null : s.id)}
                className="absolute right-2 bottom-2 flex size-6 items-center justify-center rounded-full border border-control text-muted hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
              >
                <Info size={13} aria-hidden />
              </button>
            </li>
          );
        })}
      </ul>
      {problem ? (
        <p role="alert" className="mt-2 shrink-0 text-xs text-negative">
          {problem}
        </p>
      ) : null}
    </div>
  );
}
