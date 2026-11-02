// The right-hand panel: the selected node's sentence and its tabs (Controls, Log, Results).

import { useFeed } from "../../canvas/feed";
import { plainSentence } from "../../canvas/metrics";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useDraft } from "../../data/store/draft";
import { runNow } from "../../data/store/loopRuntime";
import { useSelected } from "../../data/store/view";
import { usePickedWire } from "../../canvas/picked";
import { Controls } from "./Controls";
import { EdgeControls } from "./EdgeControls";
import { Live } from "./Live.lazy";
import { LogTab } from "./LogTab.lazy";
import { Rail } from "./Rail";
import { Results } from "./Results.lazy";
import { SystemSummary } from "./SystemSummary";
import { LiveSkeleton, LogSkeleton, ResultsSkeleton } from "./Skeletons";

export function Panel() {
  const view = useDraft((s) => s.view);
  const feed = useFeed();
  const [selected, setSelected] = useSelected();
  const config = selected !== null ? view?.doc?.nodes?.[selected] : undefined;
  const picked = usePickedWire((s) => s.index);
  const wire = selected === null && picked !== null ? view?.doc?.edges?.[picked] : undefined;
  // from the finished run, so it changes when a run lands (after a release), never mid-drag
  const sentence =
    selected !== null && config
      ? plainSentence(feed.node(selected))
      : wire
        ? `Wire from ${String(wire.from)} to ${String(wire.to)}`
        : "";

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
          ) : wire && picked !== null ? (
            <EdgeControls index={picked} />
          ) : (
            <SystemSummary />
          ),
        log: (
          <LazyBoundary fallback={<LogSkeleton />}>
            <LogTab />
          </LazyBoundary>
        ),
        live: (
          <LazyBoundary fallback={<LiveSkeleton />}>
            <Live />
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
