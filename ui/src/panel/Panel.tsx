// The right-hand panel: the selected node's sentence and its tabs (Controls, Log, Results).

import { useFeed } from "../canvas/feed";
import { plainSentence } from "../canvas/metrics";
import { LazyBoundary } from "../states/LazyBoundary";
import { useDraft } from "../store/draft";
import { runNow } from "../store/loopRuntime";
import { useSelected } from "../store/view";
import { Controls } from "./Controls";
import { LogTab } from "./LogTab.lazy";
import { Rail } from "./Rail";
import { Results } from "./Results.lazy";
import { LogSkeleton, ResultsSkeleton } from "./Skeletons";

export function Panel() {
  const view = useDraft((s) => s.view);
  const feed = useFeed();
  const [selected, setSelected] = useSelected();
  const config = selected !== null ? view?.doc?.nodes?.[selected] : undefined;
  // from the finished run, so it changes when a run lands (after a release), never mid-drag
  const sentence = selected !== null && config ? plainSentence(feed.node(selected)) : "";

  return (
    <Rail
      sentence={sentence}
      panels={{
        controls:
          selected !== null && config ? (
            <Controls
              name={selected}
              config={config}
              edges={view?.doc?.edges ?? []}
              onDeleted={() => void setSelected(null)}
              onRenamed={(to) => void setSelected(to)}
              onChaos={runNow}
            />
          ) : (
            <p className="p-3 text-sm text-muted">Select a component to see and change it.</p>
          ),
        log: (
          <LazyBoundary fallback={<LogSkeleton />}>
            <LogTab />
          </LazyBoundary>
        ),
        results: (
          <LazyBoundary fallback={<ResultsSkeleton />}>
            <Results />
          </LazyBoundary>
        ),
      }}
    />
  );
}
