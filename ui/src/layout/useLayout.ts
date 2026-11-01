// How much of Studio fits (spec 3): `full` (1280 px and up) is the layout as drawn, `rail` (768 to
// 1279) collapses the palette to an icon rail, `readonly` (under 768) is a list of the nodes, since
// editing a canvas needs room.

import { useSyncExternalStore } from "react";

export type LayoutMode = "full" | "rail" | "readonly";

export function layoutFor(width: number): LayoutMode {
  return width >= 1280 ? "full" : width >= 768 ? "rail" : "readonly";
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener("resize", onChange);
  return () => window.removeEventListener("resize", onChange);
}

export function useLayout(): LayoutMode {
  return layoutFor(
    useSyncExternalStore(
      subscribe,
      () => window.innerWidth,
      () => 1440,
    ),
  );
}

/** From this width the rail can stand in its own column from the top of the page. */
export const WIDE_PX = 1600;

export function useWide(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => window.innerWidth >= WIDE_PX,
    () => false,
  );
}
