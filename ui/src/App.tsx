// The Studio app: load the draft, keep the numbers fresh with the auto re-run loop, and show the
// top bar, canvas and panel. main.tsx only mounts this.

import { useEffect } from "react";
import { Canvas } from "./canvas/Canvas";
import { FirstRun } from "./first-run/FirstRun";
import { Loading } from "./first-run/Loading";
import { FeedContext, type StatsFeed } from "./canvas/feed";
import { Panel } from "./panel/Panel";
import { useDraft } from "./store/draft";
import { runNow, startStudioLoop } from "./store/loopRuntime";
import { useRun } from "./store/run";
import { TopBar } from "./topbar/TopBar";

export function App() {
  const view = useDraft((s) => s.view);
  const error = useDraft((s) => s.error);
  const load = useDraft((s) => s.load);
  const latest = useRun((s) => s.latest);
  const dimmed = useRun((s) => s.dimmed);

  useEffect(() => void load(), [load]);
  // the loop starts listening before the first draft arrives, so that arrival starts the first run
  useEffect(() => startStudioLoop(), []);

  if (!view && error) return <p className="p-4">{error}</p>;
  // the cards read the finished run through this feed and never import the run store
  const nodes = latest?.summary?.nodes ?? [];
  const feed: StatsFeed = { node: (name) => nodes.find((n) => n.name === name), dimmed };
  return (
    <Loading ready={view !== null}>
      <FeedContext.Provider value={feed}>
        <div className="flex h-full flex-col">
          <TopBar onRun={runNow} />
          <FirstRun />
          <div className="flex min-h-0 flex-1">
            <div className="min-w-0 flex-1">
              <Canvas />
            </div>
            <Panel />
          </div>
        </div>
      </FeedContext.Provider>
    </Loading>
  );
}
