// Ctrl/Cmd+K opens the command palette. This part is first-load code and tiny; the palette itself
// is a lazy chunk fetched on the first press.

import { lazy, useEffect, useState } from "react";
import { canvasBridge } from "../../canvas/bridge";
import { useAddNode } from "../../canvas/useAddNode";
import { useFirstRun } from "../first-run/store";
import { useGuide } from "../guide/store";
import { useDraft } from "../../data/store/draft";
import { useSelected } from "../../data/store/view";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { runNow } from "../../data/store/loopRuntime";
import { openTrafficBuilder } from "../../panels/traffic/store";
import { addNote } from "../../canvas/addNote";
import { offeredKinds } from "../../canvas/nodeKinds";
import { commands } from "./commands";

const CommandPalette = lazy(() =>
  import("./CommandPalette").then((m) => ({ default: m.CommandPalette })),
);

const Shortcuts = lazy(() => import("./Shortcuts").then((m) => ({ default: m.Shortcuts })));

export function PaletteHost() {
  const [open, setOpen] = useState(false);
  const [keys, setKeys] = useState(false);
  const addNode = useAddNode();
  const nodes = Object.keys(useDraft((s) => s.view?.doc?.nodes) ?? {});
  const [, setSelected] = useSelected();
  const replayTour = useFirstRun((s) => s.replayTour);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "?" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        // not while typing in a field or with a dialog already up
        if (
          (e.target as HTMLElement | null)?.closest?.(
            "input,textarea,select,[contenteditable=true],dialog",
          )
        )
          return;
        e.preventDefault();
        setKeys(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (keys)
    return (
      <LazyBoundary fallback={null}>
        <Shortcuts onClose={() => setKeys(false)} />
      </LazyBoundary>
    );
  if (!open) return null;
  const all = commands({
    kinds: offeredKinds(useDraft.getState().view?.kinds),
    add: addNode,
    run: runNow,
    press: (name) => document.querySelector<HTMLElement>(`[data-command="${name}"]`)?.click(),
    tour: replayTour,
    fit: () => canvasBridge.fit(),
    traffic: openTrafficBuilder,
    note: () => void addNote(),
    guide: () => useGuide.getState().show(),
    keys: () => {
      setOpen(false);
      setKeys(true);
    },
    nodes,
    select: (name) => void setSelected(name),
  });
  return (
    <LazyBoundary fallback={null}>
      <CommandPalette all={all} onClose={() => setOpen(false)} />
    </LazyBoundary>
  );
}
