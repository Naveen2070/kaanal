// The Studio app: load the draft, keep the numbers fresh with the auto re-run loop, and show the
// top bar, canvas and panel. main.tsx only mounts this.

import { useEffect } from "react";
import { canvasBridge } from "./canvas/bridge";
import { ControlDock } from "./canvas/ControlDock";
import { ZoomPill } from "./canvas/ZoomPill";
import { Canvas } from "./canvas/Canvas";
import { ChartsBand } from "./charts/ChartsBand.lazy";
import { FirstRun } from "./first-run/FirstRun";
import { Loading } from "./first-run/Loading";
import { FeedContext, type StatsFeed } from "./canvas/feed";
import { Palette } from "./canvas/Palette";
import { ReadonlyNodes } from "./layout/ReadonlyNodes";
import { Shell } from "./layout/Shell";
import { PaletteHost } from "./palette/PaletteHost";
import { Panel } from "./panel/Panel";
import { LazyBoundary } from "./states/LazyBoundary";
import { useDraft } from "./store/draft";
import { runNow, startStudioLoop } from "./store/loopRuntime";
import { useRun } from "./store/run";
import { KpiStrip } from "./topbar/KpiStrip";
import { TrafficHost } from "./traffic/TrafficHost";
import { TopBar, TopTools } from "./topbar/TopBar";
import { startNotifications } from "./toast/bind";
import { ToastHost } from "./toast/ToastHost";

export function App() {
  const view = useDraft((s) => s.view);
  const load = useDraft((s) => s.load);
  const addNode = useDraft((s) => s.addNode);
  const latest = useRun((s) => s.latest);
  const dimmed = useRun((s) => s.dimmed);

  useEffect(() => void load(), [load]);
  // the loop starts listening before the first draft arrives, so that arrival starts the first run
  useEffect(() => startStudioLoop(), []);
  // what the stores report about the state of things becomes notices
  useEffect(() => startNotifications(), []);

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
              <PaletteHost />
              <TrafficHost />
            </>
          }
          topRight={<TopTools />}
          zoom={<ZoomPill />}
          toasts={<ToastHost />}
          palette={<Palette onAdd={(kind) => void addNode(kind, canvasBridge.centre())} />}
          canvas={<Canvas />}
          dock={<ControlDock />}
          bottom={
            <LazyBoundary fallback={null}>
              <ChartsBand />
            </LazyBoundary>
          }
          rail={<Panel />}
          readonly={<ReadonlyNodes />}
        />
      </FeedContext.Provider>
    </Loading>
  );
}
