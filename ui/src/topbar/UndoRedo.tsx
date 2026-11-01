// Undo and redo (spec 9): two icon buttons and Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z (and Ctrl+Y). The keys
// leave text fields alone, where they mean the field's own undo.

import { Redo2, Undo2 } from "lucide-react";
import { useEffect } from "react";
import { useDraft } from "../store/draft";
import { useHistory } from "../store/history";

const TEXT = "input,textarea,select,[contenteditable=true],[contenteditable=''],dialog";

export function UndoRedo() {
  const canUndo = useHistory((s) => s.past.length > 0);
  const canRedo = useHistory((s) => s.future.length > 0);
  const note = useHistory((s) => s.note);
  const undo = useDraft((s) => s.undo);
  const redo = useDraft((s) => s.redo);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      if ((e.target as HTMLElement | null)?.closest?.(TEXT)) return;
      const k = e.key.toLowerCase();
      if (k === "z" && !e.shiftKey) void undo();
      else if ((k === "z" && e.shiftKey) || k === "y") void redo();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  const button =
    "flex h-8 w-8 items-center justify-center rounded-lg text-muted hover:bg-raised disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-accent";
  return (
    <div className="flex items-center gap-0.5">
      <button
        type="button"
        aria-label="Undo"
        title="Undo (Ctrl+Z)"
        disabled={!canUndo}
        onClick={() => void undo()}
        className={button}
      >
        <Undo2 size={16} aria-hidden />
      </button>
      <button
        type="button"
        aria-label="Redo"
        title="Redo (Ctrl+Shift+Z)"
        disabled={!canRedo}
        onClick={() => void redo()}
        className={button}
      >
        <Redo2 size={16} aria-hidden />
      </button>
      {note ? <output className="ml-1 text-xs text-muted">{note}</output> : null}
    </div>
  );
}
