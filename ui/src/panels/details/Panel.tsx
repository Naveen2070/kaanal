// The right-hand panel: the selected node's sentence and its tabs (Controls, Log, Results).

import { useFeed } from "../../canvas/feed";
import { plainSentence } from "../../canvas/metrics";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useDraft } from "../../data/store/draft";
import { runNow } from "../../data/store/loopRuntime";
import { isLive, useMode } from "../../data/store/mode";
import { useSelected } from "../../data/store/view";
import { usePickedWire } from "../../canvas/picked";
import { useEffect } from "react";
import { PanelBottomClose } from "lucide-react";
import { createPortal } from "react-dom";
import { AdminFeed } from "./AdminFeed";
import { Controls } from "./Controls";
import { EdgeControls } from "./EdgeControls";
import { Live } from "./Live.lazy";
import { LogTab } from "./LogTab.lazy";
import { LiveTrace } from "./LiveTrace.lazy";
import { useLogDrawer } from "./logDrawer";
import { Rail } from "./Rail";
import { LiveResults } from "./LiveResults.lazy";
import { Results } from "./Results.lazy";
import { SystemSummary } from "./SystemSummary";
import { LiveSkeleton, LogSkeleton, ResultsSkeleton } from "./Skeletons";

export function Panel() {
  const view = useDraft((s) => s.view);
  const feed = useFeed();
  const mode = useMode((s) => s.mode);
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

  const out = useLogDrawer((s) => s.out);
  const setOut = useLogDrawer((s) => s.set);
  // Escape puts the log back, unless a field or an open dialog has the key
  useEffect(() => {
    if (!out) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if ((e.target as HTMLElement | null)?.closest?.("input,select,textarea,dialog")) return;
      setOut(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [out, setOut]);
  return (
    <>
      {out
        ? createPortal(
            <section
              aria-label="Log"
              className="fixed inset-x-4 bottom-4 z-30 mx-auto flex h-[44vh] max-w-5xl flex-col overflow-hidden rounded-2xl border border-border bg-sheet shadow-float"
            >
              <div className="flex shrink-0 items-center justify-between border-b border-border px-3 py-1.5">
                <h2 className="m-0 text-sm font-semibold">Log</h2>
                <button
                  type="button"
                  onClick={() => setOut(false)}
                  className="flex items-center gap-1 rounded border border-control bg-sheet px-1.5 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <PanelBottomClose size={12} aria-hidden />
                  Bring back
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto">
                <LazyBoundary fallback={<LogSkeleton />}>
                  <LogTab drawer />
                </LazyBoundary>
              </div>
            </section>,
            document.body,
          )
        : null}
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
          log:
            mode === "admin" ? (
              <AdminFeed />
            ) : isLive(mode) ? (
              <LazyBoundary fallback={<LogSkeleton />}>
                <LiveTrace />
              </LazyBoundary>
            ) : (
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
              {isLive(mode) ? <LiveResults /> : <Results />}
            </LazyBoundary>
          ),
        }}
      />
    </>
  );
}
