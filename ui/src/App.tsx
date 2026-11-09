// The Studio app: load the draft, keep the numbers fresh with the auto re-run loop, and show the
// top bar, canvas and panel. main.tsx only mounts this.

import { useEffect } from "react";
import { useAddNode } from "./canvas/useAddNode";
import { ControlDock } from "./canvas/ControlDock";
import { ZoomPill } from "./canvas/ZoomPill";
import { Canvas } from "./canvas/Canvas";
import { ChartsBand } from "./panels/charts/ChartsBand.lazy";
import { FirstRun } from "./overlays/first-run/FirstRun";
import { Loading } from "./overlays/first-run/Loading";
import { FeedContext, type StatsFeed } from "./canvas/feed";
import { Palette } from "./canvas/Palette";
import { ReadonlyNodes } from "./shell/layout/ReadonlyNodes";
import { Shell } from "./shell/layout/Shell";
import { GuideHost } from "./overlays/guide/GuideHost";
import { PaletteHost } from "./overlays/palette/PaletteHost";
import { Panel } from "./panels/details/Panel";
import { LazyBoundary } from "./kit/LazyBoundary";
import { useDraft } from "./data/store/draft";
import { runNow } from "./data/store/loopRuntime";
import { useModeStream } from "./data/store/useModeStream";
import { useAdmin } from "./data/store/admin";
import { isLive, useMode } from "./data/store/mode";
import { useLive } from "./data/store/live";
import { liveFeed } from "./canvas/liveFeed";
import { useDotFeeder } from "./canvas/dots/dotStore";
import { useRun } from "./data/store/run";
import { KpiStrip } from "./shell/topbar/KpiStrip";
import { AdminStrip } from "./shell/topbar/AdminStrip";
import { LiveStrip } from "./shell/topbar/LiveStrip";
import { TrafficHost } from "./panels/traffic/TrafficHost";
import { TopBar, TopTools } from "./shell/topbar/TopBar";
import { startNotifications } from "./shell/toast/bind";
import { ToastHost } from "./shell/toast/ToastHost";

export function App() {
  const view = useDraft((s) => s.view);
  const load = useDraft((s) => s.load);
  const addNode = useAddNode();
  const latest = useRun((s) => s.latest);
  const dimmed = useRun((s) => s.dimmed);

  useEffect(() => void load(), [load]);
  // live (the default) runs by itself on the applied system; batch starts listening before the
  // first draft arrives, so that arrival starts the first run
  const mode = useMode((s) => s.mode);
  const ticks = useLive((s) => s.ticks);
  useDotFeeder();
  useModeStream(mode);
  // what the stores report about the state of things becomes notices
  useEffect(() => startNotifications(), []);

  // a draft that cannot be loaded is shown by <Loading> (the reason and a Retry), not here
  // the cards read the finished run through this feed and never import the run store
  const nodes = latest?.summary?.nodes ?? [];
  const links = latest?.summary?.links ?? [];
  const pipes = latest?.summary?.pipes ?? [];
  const batchFeed: StatsFeed = {
    node: (name) => nodes.find((n) => n.name === name),
    link: (from, to) => links.find((l) => l.from === from && l.to === to),
    links,
    pipes,
    nodes,
    dimmed,
    peakVisits: Math.max(0, ...nodes.map((n) => n.visits_per_s).filter(Number.isFinite)),
  };
  // live mode reads the last seconds of the live run; batch reads the finished run
  const order = useLive((s) => s.hello?.nodes);
  const realOnly = useAdmin((s) => s.realOnly) && mode === "admin";
  const feed = isLive(mode)
    ? liveFeed(ticks, view?.doc ?? null, order, realOnly ? "real" : "all")
    : batchFeed;
  return (
    <Loading ready={view !== null}>
      <FeedContext.Provider value={feed}>
        <Shell
          topLeft={<TopBar onRun={runNow} />}
          top={
            <>
              {mode === "admin" ? <AdminStrip /> : isLive(mode) ? <LiveStrip /> : <KpiStrip />}
              <FirstRun />
              <PaletteHost />
              <GuideHost />
              <TrafficHost />
            </>
          }
          topRight={<TopTools />}
          zoom={<ZoomPill />}
          toasts={<ToastHost />}
          palette={<Palette onAdd={addNode} />}
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
