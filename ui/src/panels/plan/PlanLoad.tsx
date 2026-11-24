// Load proposed draft (step 9c-3). A clean draft of the design the plan was made on is replaced at
// once; a draft with unsaved edits, or a plan made on an earlier design (changes were applied since,
// and the proposed text does not have them), gets an inline confirm first (Cancel focused), with Copy
// my draft so nothing is lost by accident. The whole-text replace is not on the Undo history, so the
// confirm says so. Kaanal never applies it.

import { useEffect, useRef, useState } from "react";
import { useDraft } from "../../data/store/draft";
import { usePlan } from "../../data/store/plan";
import { notify } from "../../shell/toast/toasts";

const btn =
  "min-h-7 rounded border border-control bg-sheet px-3 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function PlanLoad({ draft, stale }: { draft: string; stale: boolean }) {
  /** Why the confirm is open: unsaved edits, or an earlier design, or both. */
  const [confirm, setConfirm] = useState<{ dirty: boolean } | null>(null);
  const [copied, setCopied] = useState("");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const loadRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const opened = useRef(false);
  const isOpen = confirm !== null;

  useEffect(() => {
    // the confirm opens on Cancel; closing it puts focus back on Load
    if (isOpen) cancelRef.current?.focus();
    else if (opened.current) loadRef.current?.focus();
    opened.current = isOpen;
  }, [isOpen]);

  async function replace() {
    if (busy) return;
    setBusy(true);
    setProblem("");
    const ok = await useDraft.getState().replaceText(draft);
    setBusy(false);
    if (!ok) {
      const why = `Could not load the proposed draft: ${useDraft.getState().error ?? "the server refused it"}`;
      // inline (the dialog is modal, so a toast may sit behind it) and as a toast
      setProblem(why);
      notify.error(why, "plan");
      return;
    }
    usePlan.getState().setOpen(false);
    notify.success("Loaded the proposed draft. Apply it when you are ready.", "plan");
  }

  function copyMine() {
    const view = useDraft.getState().view;
    const clip = navigator.clipboard;
    if (!view) return setCopied("Nothing to copy yet");
    const text = view.text;
    if (!clip) return setCopied("Could not copy");
    clip.writeText(text).then(
      () => setCopied("Copied"),
      () => setCopied("Could not copy"),
    );
  }

  return (
    <div className="space-y-2">
      <button
        ref={loadRef}
        type="button"
        aria-expanded={confirm !== null}
        onClick={async () => {
          // let the edits still on their way settle, then read: an edit made while the dialog was open counts
          await useDraft.getState().settled();
          const dirty = useDraft.getState().view?.dirty ?? false;
          if (dirty || stale) setConfirm({ dirty });
          else void replace();
        }}
        disabled={busy}
        className={btn}
      >
        Load proposed draft
      </button>
      {problem ? (
        <p role="alert" className="m-0 text-xs text-negative">
          {problem}
        </p>
      ) : null}
      {confirm ? (
        <fieldset className="m-0 min-w-0 space-y-2 rounded-lg border border-warn p-2 text-xs">
          <legend className="px-1 font-medium">
            {confirm.dirty
              ? "This replaces your unsaved edits. They cannot be brought back with Undo."
              : "This plan was made on an earlier design."}
          </legend>
          {stale ? (
            <p className="m-0">
              {confirm.dirty
                ? "This plan was made on an earlier design."
                : "Loading it replaces your draft with that design plus the plan's changes, so changes applied since are not in it. They cannot be brought back with Undo."}
            </p>
          ) : null}
          <div className="flex flex-wrap items-center gap-2">
            <button type="button" onClick={copyMine} className={btn}>
              Copy my draft
            </button>
            <button type="button" disabled={busy} onClick={() => void replace()} className={btn}>
              Replace
            </button>
            <button ref={cancelRef} type="button" onClick={() => setConfirm(null)} className={btn}>
              Cancel
            </button>
            <output className="text-muted">{copied}</output>
          </div>
        </fieldset>
      ) : null}
    </div>
  );
}
