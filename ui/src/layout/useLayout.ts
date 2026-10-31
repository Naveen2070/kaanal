// How much of Studio fits (spec 3): `full` (1280 px and up) is the layout as drawn, `rail` (900 to
// 1279) collapses the palette to an icon rail, `readonly` (under 900) is a list of the nodes, since
// editing a canvas needs room.

import { useSyncExternalStore } from "react";

export type LayoutMode = "full" | "rail" | "readonly";

export function layoutFor(width: number): LayoutMode {
  return width >= 1280 ? "full" : width >= 900 ? "rail" : "readonly";
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
