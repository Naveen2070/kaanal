// A help button for a jargon label: a 24 px target (WCAG 2.5.8) that opens one sentence. A real
// button, so Enter and Space work and touch can reach it; Escape or a press outside closes it and
// focus returns to the button. Hand-made rather than Radix Popover: that pulls floating-ui into the
// first load and this is one sentence (the size budget, spec 11, decided it). Fixed positioning
// from the button's box, so no card or panel can clip it.

import { ArrowRight, CircleHelp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useGuide } from "../guide/store";
import { HELP, HELP_LABEL, type HelpId } from "./copy";

const WIDTH = 256;
/** The room a card needs below its button; with less, it opens above (a card is about 200 px tall). */
const ROOM = 240;

export function HelpPopover({
  id,
  label = HELP_LABEL[id],
  className = "",
}: {
  id: HelpId;
  label?: string;
  /** Extra classes for the button (a negative margin keeps a tight row its height). */
  className?: string;
}) {
  const [at, setAt] = useState<{ top?: number; bottom?: number; left: number } | null>(null);
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
        onClick={(e) => {
          // on a canvas card a press must not select the card or start a drag (React Flow's classes)
          e.stopPropagation();
          if (open) return close(false);
          const r = button.current?.getBoundingClientRect();
          if (!r) return;
          // below the whole figure it explains when it sits in one (data-help-host), so the value stays readable
          const host = button.current?.closest("[data-help-host]")?.getBoundingClientRect();
          const below = (host?.bottom ?? r.bottom) + 6;
          const left = Math.max(8, Math.min(r.left, window.innerWidth - WIDTH - 8));
          // too near the bottom of the window: open above the figure instead, so it is never clipped
          if (window.innerHeight - below < ROOM) {
            setAt({ bottom: window.innerHeight - (host?.top ?? r.top) + 6, left });
          } else setAt({ top: below, left });
        }}
        className={`nodrag nopan inline-flex size-6 shrink-0 items-center justify-center rounded-full text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent ${className}`}
      >
        <CircleHelp size={14} aria-hidden />
      </button>
      {at
        ? // on the page, not in the canvas: a card is clipped and scaled by the node it sits in
          createPortal(
            <dialog
              ref={bubble}
              open
              aria-label={label}
              style={{ top: at.top, bottom: at.bottom, left: at.left, width: WIDTH }}
              className="fixed z-50 m-0 rounded-lg border border-border bg-sheet p-3 text-xs font-normal whitespace-normal text-text"
            >
              <p className="m-0">{HELP[id].what}</p>
              {HELP[id].example ? (
                <p className="m-0 mt-2 text-muted">
                  <span className="font-medium text-text">Example:</span> {HELP[id].example}
                </p>
              ) : null}
              {HELP[id].hint ? (
                <p className="m-0 mt-1 text-muted">
                  <span className="font-medium text-text">Hint:</span> {HELP[id].hint}
                </p>
              ) : null}
              <button
                type="button"
                onClick={() => {
                  useGuide.getState().show({ entry: HELP[id].entry ?? id });
                  close(false);
                }}
                className="mt-2 inline-flex items-center gap-1 rounded-sm text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent"
              >
                Open {label} in the glossary
                <ArrowRight size={12} aria-hidden />
              </button>
            </dialog>,
            document.body,
          )
        : null}
    </>
  );
}
