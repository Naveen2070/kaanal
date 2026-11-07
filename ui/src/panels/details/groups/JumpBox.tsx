// "Jump to a setting": type a word, pick a match, and its group opens and the box takes focus. For a
// power user who knows what they want in a panel of folded groups. The matches are a plain list under
// the box (no floating menu to clip), Enter takes the first, Escape clears.

import { Search } from "lucide-react";
import { useState } from "react";
import { type JumpTarget, matchTargets } from "./jump";
import { useGroups } from "./store";

/** Focuses the control of the label with this exact text; false when none is on the page. */
function focusLabel(label: string): boolean {
  const el = [...document.querySelectorAll("label")].find((l) => l.textContent?.trim() === label);
  const control = el?.control;
  if (!control) return false;
  control.scrollIntoView?.({ block: "center" });
  control.focus();
  return true;
}

export function JumpBox({ targets }: { targets: readonly JumpTarget[] }) {
  const [text, setText] = useState("");
  const found = matchTargets(targets, text);
  const go = (t: JumpTarget) => {
    if (t.group) useGroups.getState().set(t.group, true);
    setText("");
    // after the group has rendered its boxes
    setTimeout(() => {
      if (!focusLabel(t.label) && t.fallback) focusLabel(t.fallback);
    }, 0);
  };
  return (
    <div className="w-40 shrink-0">
      <label className="relative block">
        <span className="sr-only">Jump to a setting</span>
        <Search
          size={14}
          aria-hidden
          className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-muted"
        />
        <input
          type="search"
          name="jump-to-a-setting"
          placeholder="Jump to…"
          value={text}
          onChange={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && found[0]) go(found[0]);
            else if (e.key === "Escape" && text) {
              // clears the box, and stays out of the panel's own Escape
              e.stopPropagation();
              setText("");
            }
          }}
          className="h-8 w-full rounded-md border border-control bg-sheet pr-2 pl-7 text-sm"
        />
      </label>
      {text.trim() ? (
        <div aria-live="polite" className="mt-1">
          {found.length === 0 ? (
            <p className="m-0 px-1 text-xs text-muted">No setting matches.</p>
          ) : (
            <ul className="m-0 list-none p-0">
              {found.map((t) => (
                <li key={t.label}>
                  <button
                    type="button"
                    onClick={() => go(t)}
                    className="w-full rounded-sm px-1.5 py-1 text-left text-sm hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
                  >
                    {t.label}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}
