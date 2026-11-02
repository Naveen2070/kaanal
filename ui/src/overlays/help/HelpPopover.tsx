// A help button for a jargon label: a 24 px target (WCAG 2.5.8) that opens one sentence. A real
// button, so Enter and Space work and touch can reach it; Escape or a press outside closes it and
// focus returns to the button. Hand-made rather than Radix Popover: that pulls floating-ui into the
// first load and this is one sentence (the size budget, spec 11, decided it). Fixed positioning
// from the button's box, so no card or panel can clip it.

import { CircleHelp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { HELP, HELP_LABEL, type HelpId } from "./copy";

const WIDTH = 256;

export function HelpPopover({ id, label = HELP_LABEL[id] }: { id: HelpId; label?: string }) {
  const [at, setAt] = useState<{ top: number; left: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const bubble = useRef<HTMLDialogElement>(null);
  const open = at !== null;

  const close = (refocus: boolean) => {
    setAt(null);
    if (refocus) button.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // captured and stopped: Escape closes the help, not the control bar or panel it sits in
      e.preventDefault();
      e.stopPropagation();
      close(true);
    };
    const onDown = (e: PointerEvent | MouseEvent) => {
      const t = e.target as Node;
      if (!bubble.current?.contains(t) && !button.current?.contains(t)) close(false);
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown);
    };
  }, [open]);

  return (
    <>
      <button
        ref={button}
        type="button"
        aria-label={`About ${label}`}
        aria-expanded={open}
        onClick={() => {
          if (open) return close(false);
          const r = button.current?.getBoundingClientRect();
          if (!r) return;
          // below the whole figure it explains when it sits in one (data-help-host), so the value stays readable
          const host = button.current?.closest("[data-help-host]")?.getBoundingClientRect();
          setAt({
            top: (host?.bottom ?? r.bottom) + 6,
            left: Math.max(8, Math.min(r.left, window.innerWidth - WIDTH - 8)),
          });
        }}
        className="inline-flex size-6 shrink-0 items-center justify-center rounded-full text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <CircleHelp size={14} aria-hidden />
      </button>
      {at ? (
        <dialog
          ref={bubble}
          open
          aria-label={label}
          style={{ top: at.top, left: at.left, width: WIDTH }}
          className="fixed z-50 m-0 rounded-lg border border-border bg-sheet p-3 text-xs font-normal whitespace-normal text-text"
        >
          {HELP[id]}
        </dialog>
      ) : null}
    </>
  );
}
