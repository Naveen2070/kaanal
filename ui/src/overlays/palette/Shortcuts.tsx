// The keyboard shortcuts sheet (review C6): a modal dialog listing the keys that really exist.
// Escape closes it (native dialog) and focus goes back to where it was. A separate lazy chunk.

import { useLayoutEffect, useRef } from "react";

export const SHORTCUTS: [keys: string, what: string][] = [
  ["Ctrl K", "Open the command palette"],
  ["Ctrl Z", "Undo"],
  ["Ctrl Shift Z", "Redo"],
  ["Arrow keys", "Move the selected component 8 px"],
  ["Shift + arrow keys", "Move the selected component 40 px"],
  ["Delete", "Remove the selected wire"],
  ["]", "Fold or open the details panel"],
  ["Esc", "Close the innermost open layer"],
  ["?", "Show this list"],
];

export function Shortcuts({ onClose }: { onClose: () => void }) {
  const box = useRef<HTMLDialogElement>(null);

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

  return (
    <dialog
      ref={box}
      aria-label="Keyboard shortcuts"
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className="fixed top-24 left-1/2 m-0 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 rounded-2xl border border-border bg-sheet p-4 text-sm text-text backdrop:bg-black/30"
    >
      <h2 className="mb-2 text-base font-semibold">Keyboard shortcuts</h2>
      <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
        {SHORTCUTS.map(([keys, what]) => (
          <div key={keys} className="contents">
            <dt>
              <kbd className="rounded border border-control px-1.5 font-mono text-xs">{keys}</kbd>
            </dt>
            <dd className="m-0 text-muted">{what}</dd>
          </div>
        ))}
      </dl>
      <button
        type="button"
        onClick={onClose}
        className="mt-3 h-8 rounded-lg border border-control px-3"
      >
        Close
      </button>
    </dialog>
  );
}
