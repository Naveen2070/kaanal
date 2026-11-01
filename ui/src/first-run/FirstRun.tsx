// Decides, once the draft is loaded, whether the welcome card or the tour is on screen.

import { lazy, useEffect } from "react";
import { LazyBoundary } from "../states/LazyBoundary";
import { useDraft } from "../store/draft";
import { projectKeyOf } from "./flags";
import { useFirstRun } from "./store";
import { Welcome } from "./Welcome";

// the tour is fetched when it first shows, not with the first load
const Tour = lazy(() => import("./Tour").then((m) => ({ default: m.Tour })));

export function FirstRun() {
  const projectId = useDraft((s) => s.view?.project_id);
  const phase = useFirstRun((s) => s.phase);
  const begin = useFirstRun((s) => s.begin);
  useEffect(() => {
    // new = a draft with no components yet; read at this moment, not on every edit
    const nodes = useDraft.getState().view?.doc?.nodes ?? {};
    begin(projectKeyOf(projectId), Object.keys(nodes).length === 0);
  }, [begin, projectId]);
  if (phase === "welcome") return <Welcome />;
  if (phase === "tour") {
    return (
      <LazyBoundary fallback={null}>
        <Tour />
      </LazyBoundary>
    );
  }
  return null;
}
