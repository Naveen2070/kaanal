// Keyboard and screen-reader help for the step lists: Alt+Up and Alt+Down move the step whose row has
// focus, what happened is said once in a polite live region, and focus follows the step (or lands on a
// neighbour when it is deleted) so a person never loses their place after the list redraws.

import { createContext } from "react";
import type { Path } from "./model";

/** Says a sentence in the group's live region. */
export const Announce = createContext<(message: string) => void>(() => {});

/** The attribute a row carries, so focus can find it again by where it sits in the document. */
export const pathKey = (path: Path): string => path.join("/");

/** The path of the nearest sibling of the step at `path` that is on screen: the same place, else the one before. */
export function siblingPaths(path: Path): Path[] {
  const last = path[path.length - 1];
  if (typeof last !== "number") return [];
  const at = path.slice(0, -1);
  return last > 0 ? [path, [...at, last - 1]] : [path];
}

/** Focuses the first row found at one of `paths`, after the list has redrawn; falls back to `fallback`. */
export function focusRow(paths: Path[], fallback?: () => HTMLElement | null): void {
  requestAnimationFrame(() => {
    for (const p of paths) {
      const el = document.querySelector<HTMLElement>(
        `[data-step-path="${CSS.escape(pathKey(p))}"] [data-row-focus]`,
      );
      if (el) return el.focus();
    }
    fallback?.()?.focus();
  });
}

/** Whether a key press is Alt+Up or Alt+Down, and which way. */
export function moveKey(e: {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  key: string;
}): -1 | 1 | 0 {
  if (!e.altKey || e.ctrlKey || e.metaKey) return 0;
  return e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
}
