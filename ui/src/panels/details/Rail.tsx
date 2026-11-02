// The right rail (spec 6): the selected node's sentence above Radix tabs. One width on every tab, in
// the row beside the canvas (never over it), and it folds to a slim handle with the button or the
// ] key, so a narrow window can have the room back.

import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../kit/tabs";
import { RAIL_FOLDED_W, RAIL_W, useRail } from "../../shell/layout/rail";
import { TABS, useTab, type Tab } from "../../data/store/view";

const LABEL: Record<Tab, string> = {
  controls: "Controls",
  log: "Log",
  results: "Results",
  live: "Live",
};

export function Rail({
  sentence,
  panels,
}: {
  /** One line about the selected node; announced politely. Empty when nothing is selected. */
  sentence: string;
  panels: Record<Tab, ReactNode>;
}) {
  const [tab, setTab] = useTab();
  const open = useRail((s) => s.open);
  const setOpen = useRail((s) => s.set);
  // `]` folds and opens it, unless a field has the keys
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "]" || e.ctrlKey || e.metaKey || e.altKey) return;
      if (
        (e.target as HTMLElement | null)?.closest?.("input,textarea,select,[contenteditable=true]")
      )
        return;
      setOpen(!useRail.getState().open);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);
  if (!open) {
    return (
      <div data-tour="panel" className="shrink-0" style={{ width: RAIL_FOLDED_W }}>
        <button
          type="button"
          aria-label="Open the details panel"
          aria-keyshortcuts="]"
          title="Open the details panel (])"
          onClick={() => setOpen(true)}
          className="flex size-11 items-center justify-center rounded-2xl border border-border bg-sheet text-muted shadow-float hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <PanelRightOpen size={18} aria-hidden />
        </button>
      </div>
    );
  }
  return (
    <div data-tour="panel" className="relative shrink-0" style={{ width: RAIL_W }}>
      <aside
        aria-label="Details"
        className="flex h-full flex-col overflow-hidden rounded-2xl border border-border bg-sheet shadow-float"
      >
        {/* a live region must exist before its words do; it changes when a run finishes, not while a slider moves */}
        <div className="flex shrink-0 items-start border-b border-border">
          <p
            data-testid="rail-sentence"
            aria-live="polite"
            className="min-h-9 min-w-0 flex-1 px-3 py-2.5 text-sm font-medium"
          >
            {sentence}
          </p>
          <button
            type="button"
            aria-label="Fold the details panel"
            aria-keyshortcuts="]"
            title="Fold the details panel (])"
            onClick={() => setOpen(false)}
            className="m-1 flex size-8 shrink-0 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
          >
            <PanelRightClose size={16} aria-hidden />
          </button>
        </div>
        <Tabs
          value={tab}
          onValueChange={(t) => void setTab(t as Tab)}
          className="flex min-h-0 flex-1 flex-col"
        >
          <TabsList aria-label="Details sections">
            {TABS.map((t) => (
              <TabsTrigger key={t} value={t}>
                {LABEL[t]}
              </TabsTrigger>
            ))}
          </TabsList>
          {TABS.map((t) => (
            <TabsContent key={t} value={t}>
              {panels[t]}
            </TabsContent>
          ))}
        </Tabs>
      </aside>
    </div>
  );
}
