// What the command palette can do (spec 7): add a component, run, apply, save, switch theme, replay
// the tour, fit the view. Pure data over a context of callbacks, so it is tested without the page.

import { NODE_KINDS } from "../canvas/nodeKinds";

export type Command = { id: string; label: string; shortcut?: string; run: () => void };

export interface CommandContext {
  add: (kind: string) => void;
  run: () => void;
  /** Press a button elsewhere on the page by its `data-command` name (apply, save, theme). */
  press: (name: "apply" | "save" | "theme") => void;
  tour: () => void;
  fit: () => void;
  traffic: () => void;
  /** The components on the canvas, to jump to by name. */
  nodes?: string[];
  select?: (name: string) => void;
}

export function commands(ctx: CommandContext): Command[] {
  return [
    ...NODE_KINDS.map((k) => ({
      id: `add-${k.kind}`,
      label: `Add ${/^[aeiou]/i.test(k.label) ? "an" : "a"} ${k.label.toLowerCase()}`,
      run: () => ctx.add(k.kind),
    })),
    { id: "run", label: "Run now", run: ctx.run },
    { id: "apply", label: "Apply the draft", run: () => ctx.press("apply") },
    { id: "save", label: "Save to kaanal.yaml", run: () => ctx.press("save") },
    { id: "theme", label: "Switch theme", run: () => ctx.press("theme") },
    { id: "tour", label: "Replay the tour", run: ctx.tour },
    { id: "fit", label: "Fit the view", run: ctx.fit },
    { id: "traffic", label: "Build traffic", run: ctx.traffic },
    ...(ctx.nodes ?? []).map((n) => ({
      id: `go-${n}`,
      label: `Go to ${n}`,
      run: () => ctx.select?.(n),
    })),
  ];
}

/** Commands whose label contains every word typed, case-insensitively; all of them for no text. */
export function filterCommands(all: Command[], text: string): Command[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  return all.filter((c) => words.every((w) => c.label.toLowerCase().includes(w)));
}
