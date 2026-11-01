// Ctrl/Cmd+K opens the command palette. This part is first-load code and tiny; the palette itself
// is a lazy chunk fetched on the first press.

import { lazy, useEffect, useState } from "react";
import { canvasBridge } from "../canvas/bridge";
import { useAddNode } from "../canvas/useAddNode";
import { useFirstRun } from "../first-run/store";
import { useDraft } from "../store/draft";
import { useSelected } from "../store/view";
import { LazyBoundary } from "../states/LazyBoundary";
import { runNow } from "../store/loopRuntime";
import { openTrafficBuilder } from "../traffic/store";
import { commands } from "./commands";

const CommandPalette = lazy(() =>
  import("./CommandPalette").then((m) => ({ default: m.CommandPalette })),
);

export function PaletteHost() {
  const [open, setOpen] = useState(false);
  const addNode = useAddNode();
  const nodes = Object.keys(useDraft((s) => s.view?.doc?.nodes) ?? {});
  const [, setSelected] = useSelected();
  const replayTour = useFirstRun((s) => s.replayTour);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!open) return null;
  const all = commands({
    add: addNode,
    run: runNow,
    press: (name) => document.querySelector<HTMLElement>(`[data-command="${name}"]`)?.click(),
    tour: replayTour,
    fit: () => canvasBridge.fit(),
    traffic: openTrafficBuilder,
    nodes,
    select: (name) => void setSelected(name),
  });
  return (
    <LazyBoundary fallback={null}>
      <CommandPalette all={all} onClose={() => setOpen(false)} />
    </LazyBoundary>
  );
}
