// The starters shown on an empty canvas, under "Start with a gateway": pick one and its YAML replaces
// the (empty) draft. A refusal is shown beside the list; nothing is lost, the draft was empty.

import { useState } from "react";
import { useDraft } from "../data/store/draft";
import { STARTERS } from "./starters";

export function StarterList() {
  const replaceText = useDraft((s) => s.replaceText);
  const [busy, setBusy] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
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
    <div className="pointer-events-auto mt-4 w-[min(28rem,90vw)] rounded-2xl border border-border bg-sheet p-3 text-sm shadow-float">
      <h2 className="m-0 mb-2 text-sm font-semibold">Or start from a system</h2>
      <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
        {STARTERS.map((s) => (
          <li key={s.id}>
            <button
              type="button"
              disabled={busy !== null}
              onClick={() => void load(s.id)}
              className="w-full rounded-lg border border-border bg-card px-3 py-2 text-left hover:bg-raised disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-accent"
            >
              <span className="block font-medium">{s.title}</span>
              <span className="block text-xs text-muted">{s.blurb}</span>
              <span className="block text-xs text-muted">Try: {s.tryThis}</span>
            </button>
          </li>
        ))}
      </ul>
      {problem ? (
        <p role="alert" className="mt-2 text-xs text-negative">
          {problem}
        </p>
      ) : null}
    </div>
  );
}
