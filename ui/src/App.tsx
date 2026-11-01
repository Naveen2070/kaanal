// The Studio app: load the draft, keep the numbers fresh with the auto re-run loop, and show the
// top bar, canvas and panel. main.tsx only mounts this.

import { Suspense, useEffect } from "react";
import { Canvas } from "./canvas/Canvas";
import { ChartsBand } from "./charts/ChartsBand.lazy";
import { FirstRun } from "./first-run/FirstRun";
import { Loading } from "./first-run/Loading";
import { FeedContext, type StatsFeed } from "./canvas/feed";
import { Palette } from "./canvas/Palette";
import { ReadonlyNodes } from "./layout/ReadonlyNodes";
import { Shell } from "./layout/Shell";
import { Panel } from "./panel/Panel";
import { useDraft } from "./store/draft";
import { runNow, startStudioLoop } from "./store/loopRuntime";
import { useRun } from "./store/run";
import { KpiStrip } from "./topbar/KpiStrip";
import { TopBar } from "./topbar/TopBar";

export function App() {
  const view = useDraft((s) => s.view);
  const load = useDraft((s) => s.load);
  const addNode = useDraft((s) => s.addNode);
  const latest = useRun((s) => s.latest);
  const dimmed = useRun((s) => s.dimmed);

  useEffect(() => void load(), [load]);
  // the loop starts listening before the first draft arrives, so that arrival starts the first run
  useEffect(() => startStudioLoop(), []);

  // a draft that cannot be loaded is shown by <Loading> (the reason and a Retry), not here
  // the cards read the finished run through this feed and never import the run store
  const nodes = latest?.summary?.nodes ?? [];
  const feed: StatsFeed = {
    node: (name) => nodes.find((n) => n.name === name),
    dimmed,
    peakVisits: Math.max(0, ...nodes.map((n) => n.visits_per_s).filter(Number.isFinite)),
  };
  return (
    <Loading ready={view !== null}>
      <FeedContext.Provider value={feed}>
        <Shell
          top={
            <>
              <TopBar onRun={runNow} />
              <KpiStrip />
              <FirstRun />
            </>
          }
          palette={<Palette onAdd={(kind) => void addNode(kind)} />}
          canvas={
            <div className="flex h-full flex-col">
              <div className="relative min-h-0 flex-1">
                <Canvas />
              </div>
              <Suspense fallback={<div className="h-9 shrink-0 border-t border-border" />}>
                <ChartsBand />
              </Suspense>
            </div>
          }
          rail={<Panel />}
          readonly={<ReadonlyNodes />}
        />
      </FeedContext.Provider>
    </Loading>
  );
}
