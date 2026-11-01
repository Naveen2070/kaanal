// The right rail (spec 6): the selected node's sentence above Radix tabs. 300 px on Controls, 440 px
// on Log and Results (animated, instant under reduced motion). Under 1280 px the widened rail floats
// over the canvas instead of pushing it, so the canvas never jumps.

import type { ReactNode } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../components/ui/tabs";
import { useLayout } from "../layout/useLayout";
import { TABS, useTab, type Tab } from "../store/view";

const LABEL: Record<Tab, string> = { controls: "Controls", log: "Log", results: "Results" };

export function Rail({
  sentence,
  panels,
}: {
  /** One line about the selected node; announced politely. Empty when nothing is selected. */
  sentence: string;
  panels: Record<Tab, ReactNode>;
}) {
  const [tab, setTab] = useTab();
  const mode = useLayout();
  const width = tab === "controls" ? 300 : 440;
  const overlay = mode !== "full";
  return (
    // the 300 px slot keeps its place in the row; an overlaid rail grows leftwards out of it
    <div
      data-tour="panel"
      className="relative shrink-0 transition-[width] duration-200 motion-reduce:transition-none"
      style={{ width: overlay ? 300 : width }}
    >
      <aside
        aria-label="Details"
        style={{ width }}
        className={`flex flex-col overflow-hidden rounded-2xl border border-border bg-sheet transition-[width] duration-200 motion-reduce:transition-none ${
          overlay ? "absolute inset-y-0 right-0 z-10" : "h-full"
        }`}
      >
        {/* a live region must exist before its words do; it changes when a run finishes, not while a slider moves */}
        <p
          data-testid="rail-sentence"
          aria-live="polite"
          className="min-h-9 shrink-0 px-3 py-2 text-sm font-medium"
        >
          {sentence}
        </p>
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
