// The right rail (spec 6): the selected node's sentence above Radix tabs. One width on every tab, in
// the row beside the canvas (never over it), and it folds to a slim handle with the button or the
// ] key, so a narrow window can have the room back.

import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../kit/tabs";
import { RAIL_FOLDED_W, RAIL_MAX_W, RAIL_MIN_W, useRail } from "../../shell/layout/rail";
import { TABS, useTab, type Tab } from "../../data/store/view";

const LABEL: Record<Tab, string> = {
  controls: "Settings",
  log: "Trace",
  results: "Results",
  live: "Try live",
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
  const width = useRail((s) => s.width);
  const setWidth = useRail((s) => s.setWidth);
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
    <div data-tour="panel" className="relative flex max-h-full shrink-0 flex-col" style={{ width }}>
      {/* drag the left edge to widen (the Log table likes room); arrow keys do the same */}
      {/* a focusable splitter is valid ARIA; the rule would have it be a plain hr */}
      <div
        // eslint-disable-next-line jsx-a11y/prefer-tag-over-role
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the details panel"
        aria-valuemin={RAIL_MIN_W}
        aria-valuemax={RAIL_MAX_W}
        aria-valuenow={width}
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setWidth(width + 24);
          else if (e.key === "ArrowRight") setWidth(width - 24);
          else return;
          e.preventDefault();
        }}
        onPointerDown={(e) => {
          const start = { x: e.clientX, w: width };
          const el = e.currentTarget;
          el.setPointerCapture(e.pointerId);
          const move = (ev: PointerEvent) => setWidth(start.w + (start.x - ev.clientX));
          const up = () => {
            el.removeEventListener("pointermove", move);
            el.removeEventListener("pointerup", up);
          };
          el.addEventListener("pointermove", move);
          el.addEventListener("pointerup", up);
        }}
        className="absolute inset-y-3 -left-1.5 z-10 w-3 cursor-col-resize touch-none rounded-full hover:bg-accent/30 focus-visible:bg-accent/40 focus-visible:outline-none"
      />
      <aside
        aria-label="Details"
        className="flex min-h-0 flex-col overflow-hidden rounded-2xl border border-border bg-sheet shadow-float"
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
