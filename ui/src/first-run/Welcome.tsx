// The welcome card in front of the canvas. It is a modal dialog: focus moves into it, Tab stays
// inside it, Escape closes it. "Try the demo" replaces the draft with Studio's starter system.

import { useEffect, useId, useRef, useState } from "react";
import { ApiError, api } from "../api/client";
import { useDraft } from "../store/draft";
import { useFirstRun } from "./store";

const STEPS = [
  ["Draw", "Drag components onto the canvas and connect them."],
  ["Break", "Slow something down, take it out, and see what gives first."],
  ["Run", "Every change re-runs the simulation on your draft."],
  ["Save", "Keep the system as kaanal.yaml when you are happy with it."],
] as const;

export function Welcome() {
  const closeWelcome = useFirstRun((s) => s.closeWelcome);
  const never = useFirstRun((s) => s.never);
  const replaceText = useDraft((s) => s.replaceText);
  const titleId = useId();
  const box = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  // a real modal: the rest of the page is inert and Tab stays inside (the browser does both);
  // without `showModal` (a test DOM) it is shown as a plain open dialog
  useEffect(() => {
    const d = box.current;
    if (!d || d.open) return;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    d.querySelector<HTMLElement>("button")?.focus();
  }, []);

  const tryDemo = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const { text } = await api.demo();
      // through the draft store, so it lands after any edit still on its way
      if (!(await replaceText(text)))
        throw new Error(useDraft.getState().error ?? "the demo was refused");
      // the card may have been dismissed while this ran: do not start a tour behind that choice
      if (useFirstRun.getState().phase === "welcome") closeWelcome();
    } catch (e) {
      setProblem(e instanceof ApiError ? e.message : String(e));
      setBusy(false);
    }
  };

  const trap = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      // handled here (not left to `cancel`) so the tour behind it can tell it was already used
      e.preventDefault();
      if (!busy) closeWelcome();
      return;
    }
    // a modal dialog traps Tab itself; the manual wrap covers the plain-dialog fallback
    if (e.key !== "Tab") return;
    const all = Array.from(
      box.current?.querySelectorAll<HTMLElement>("button:not(:disabled)") ?? [],
    );
    const first = all[0];
    const last = all[all.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <>
      <dialog
        ref={box}
        aria-modal="true"
        onCancel={(e) => {
          // Escape: closes like Start blank, but not while the demo is loading
          e.preventDefault();
          if (!busy) closeWelcome();
        }}
        aria-labelledby={titleId}
        onKeyDown={trap}
        className="w-full max-w-md rounded-lg border-0 bg-white p-5 text-neutral-900 shadow-xl backdrop:bg-black/40 dark:bg-neutral-900 dark:text-neutral-100"
      >
        <h2 id={titleId} className="mb-3 text-lg font-semibold">
          Welcome to Kaanal Studio
        </h2>
        <ol className="mb-4 space-y-1 text-sm">
          {STEPS.map(([title, text], i) => (
            <li key={title}>
              <span className="font-medium">
                {i + 1}. {title}
              </span>{" "}
              <span className="text-neutral-600 dark:text-neutral-300">{text}</span>
            </li>
          ))}
        </ol>
        {problem ? (
          <p role="alert" className="mb-2 text-sm text-red-700">
            {problem}
          </p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="rounded border px-3 py-1"
            disabled={busy}
            onClick={closeWelcome}
          >
            Start blank
          </button>
          <button
            type="button"
            className="rounded border border-blue-600 bg-blue-600 px-3 py-1 text-white"
            disabled={busy}
            onClick={() => void tryDemo()}
          >
            Try the demo
          </button>
          <button
            type="button"
            className="ml-auto text-sm underline"
            disabled={busy}
            onClick={never}
          >
            Don't show again
          </button>
        </div>
      </dialog>
    </>
  );
}
