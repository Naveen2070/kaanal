// The command palette (spec 7): a modal dialog with a combobox over a listbox. Typing filters,
// the arrow keys move the active option (announced through aria-activedescendant), Enter runs it,
// Escape closes and focus goes back to where it was. The native dialog traps focus. A separate chunk.

import { useId, useLayoutEffect, useRef, useState } from "react";
import { filterCommands, type Command } from "./commands";

export function CommandPalette({ all, onClose }: { all: Command[]; onClose: () => void }) {
  const box = useRef<HTMLDialogElement>(null);
  const listId = useId();
  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  const shown = filterCommands(all, text);
  const at = Math.min(active, Math.max(shown.length - 1, 0));

  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);

  const runAt = (i: number) => {
    const c = shown[i];
    if (!c) return;
    onClose();
    c.run();
  };

  return (
    <dialog
      ref={box}
      aria-label="Command palette"
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className="fixed top-24 left-1/2 m-0 w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 rounded-2xl border border-border bg-sheet p-2 text-sm text-text backdrop:bg-black/30"
    >
      <input
        type="text"
        role="combobox"
        aria-label="Type a command"
        aria-expanded="true"
        aria-controls={listId}
        aria-activedescendant={shown.length > 0 ? `${listId}-${at}` : undefined}
        aria-autocomplete="list"
        value={text}
        placeholder="Type a command"
        onChange={(e) => {
          setText(e.currentTarget.value);
          setActive(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive((at + 1) % Math.max(shown.length, 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((at - 1 + shown.length) % Math.max(shown.length, 1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            runAt(at);
          }
        }}
        className="mb-1 h-9 w-full rounded-lg border border-control bg-sheet px-2"
      />
      {/* a native listbox: the input keeps focus and points at the active option */}
      <select
        id={listId}
        size={Math.max(shown.length, 2)}
        aria-label="Commands"
        value={shown[at]?.id ?? ""}
        onChange={() => {}}
        tabIndex={-1}
        className="max-h-72 w-full overflow-y-auto border-0 bg-sheet outline-none"
      >
        {shown.map((c, i) => (
          <option
            key={c.id}
            id={`${listId}-${i}`}
            value={c.id}
            onClick={() => runAt(i)}
            className={`h-8 rounded-lg px-2 ${i === at ? "bg-active" : ""}`}
          >
            {c.label}
          </option>
        ))}
      </select>
      {shown.length === 0 ? <p className="p-2 text-muted">No command matches.</p> : null}
    </dialog>
  );
}
