// A caption on the canvas: a small card of plain text you can type into, drag, and delete. It is
// not a component (no wires, no figures), so it looks like paper beside the cards. The text is kept
// when the box loses focus, not on every key.

import type { NodeProps } from "@xyflow/react";
import { X } from "lucide-react";
import { MAX_NOTE_CHARS, useNotes } from "./notes";

export function NoteCard({ id, data }: NodeProps) {
  const text = (data as { text?: string }).text ?? "";
  const setText = useNotes((s) => s.setText);
  const remove = useNotes((s) => s.remove);
  return (
    <div className="group relative w-52 rounded-xl border border-dashed border-control bg-raised p-2 shadow-float">
      <textarea
        // `nodrag` and `nowheel`: selecting text must not drag the card, scrolling it must not zoom
        className="nodrag nowheel block h-20 w-full resize-none bg-transparent text-sm italic text-text outline-none placeholder:text-muted"
        aria-label="Note"
        placeholder="Write a note..."
        maxLength={MAX_NOTE_CHARS}
        defaultValue={text}
        key={text}
        onBlur={(e) => {
          if (e.currentTarget.value !== text) setText(id, e.currentTarget.value);
        }}
      />
      <button
        type="button"
        aria-label="Delete this note"
        title="Delete this note"
        onClick={() => remove(id)}
        className="nodrag absolute -top-2 -right-2 flex size-5 items-center justify-center rounded-full border border-control bg-sheet text-muted opacity-0 group-hover:opacity-100 hover:text-text focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent"
      >
        <X size={12} aria-hidden />
      </button>
    </div>
  );
}
